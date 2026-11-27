{ config, lib, pkgs, gb-grid-pkg, ... }@args:

let
  # Optional packages, passed via specialArgs. (Read through `args`: the module
  # system ignores `? null` defaults on module function arguments, so a missing
  # one would fail evaluation instead of falling back.)
  gb-grid-ingest-pkg = args.gb-grid-ingest-pkg or null;
  gb-grid-web-pkg = args.gb-grid-web-pkg or null;

  cfg = config.services.gb-grid;
  dbName = "gb_grid";
  dbUser = "gb_grid";
  appHome = "/var/lib/gb-grid";
  dashboardsDir = ./../grafana/dashboards;

  # The Rust ingester (pass via specialArgs `gb-grid-ingest-pkg`) replaces the
  # Python scheduler/ingest side; migrations still run through the Python
  # package (0004 is a Python yoyo step). Falls back to the Python ingester
  # when the Rust package isn't supplied, so existing consumers keep working.
  ingestBin = if gb-grid-ingest-pkg != null
    then "${gb-grid-ingest-pkg}/bin/gb-grid-ingest"
    else "${gb-grid-pkg}/bin/gb-grid";
  migrateBin = "${gb-grid-pkg}/bin/gb-grid";

  # TimescaleDB is under the (unfree) TSL licence. Allow just that one package
  # via a scoped pkgs import so consumers don't need host-wide allowUnfree.
  tsPkgs = import pkgs.path {
    inherit (pkgs) system;
    config = pkgs.config // {
      allowUnfreePredicate = p: builtins.elem (lib.getName p) [ "timescaledb" ];
    };
  };
  # TimescaleDB older than 2.32.1 breaks on PostgreSQL >= 16.16 / 17.12 / 18.7.
  # Every job the background scheduler runs (CAGG refresh and compression
  # policies) is recorded as failed with `cannot drop portal "" while it is
  # being set up` and backed off (timescale/timescaledb#11038, fixed by #11015
  # in 2.32.1). nixos-26.05 ships 2.27.1 with PostgreSQL 16.16, so until the
  # pinned nixpkgs has >= 2.32.1, build 2.32.1 with nixpkgs' own derivation
  # (master's recipe differs only in version and hash). This turns itself
  # off once nixpkgs catches up.
  timescaledbFor = p:
    if lib.versionAtLeast p.timescaledb.version "2.32.1" then p.timescaledb
    else p.timescaledb.overrideAttrs (old: {
      version = "2.32.1";
      src = old.src.override {
        tag = "2.32.1";
        hash = "sha256-UzYCXd+F4pVC1q1gmrU1OakgLDYxO3mu6TkZnuAKuAg=";
      };
    });
  pgPackage = tsPkgs.postgresql_16.withPackages (p: [ (timescaledbFor p) ]);

  # After a TimescaleDB package change, every session in the DB fails with
  # `could not access file "$libdir/timescaledb-<old>"` until the extension is
  # updated, and the update must be the session's first statement. So run it
  # before migrate on every start. It's a no-op when already current (just a
  # notice), and skipped on a fresh DB where migrate hasn't created the
  # extension yet. Runs as gb_grid, the extension's owner.
  timescaledbUpdate = pkgs.writeShellScript "gb-grid-timescaledb-update" ''
    out=$(${pgPackage}/bin/psql -X -d ${dbName} -v ON_ERROR_STOP=1 \
      -c 'ALTER EXTENSION timescaledb UPDATE' 2>&1) && { echo "$out"; exit 0; }
    case "$out" in
      *'extension "timescaledb" does not exist'*) echo "timescaledb not installed yet; skipping update"; exit 0 ;;
    esac
    echo "$out" >&2
    exit 1
  '';
in {
  options.services.gb-grid = {
    enable = lib.mkEnableOption "GB-grid ingester, Postgres, and Grafana";

    grafanaPort = lib.mkOption {
      type = lib.types.port;
      default = 3000;
    };

    openGrafanaFirewall = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Open the Grafana port in the host firewall.";
    };

    web = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = gb-grid-web-pkg != null;
        defaultText = lib.literalExpression "gb-grid-web-pkg != null";
        description = ''
          Serve the web UI (pass packages.gb-grid-web as specialArgs
          `gb-grid-web-pkg`): nginx on `web.port` serves the static build and
          proxies /api to PostgREST, which exposes the database's `api` schema.
        '';
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 8080;
        description = "Port nginx serves the web UI on.";
      };

      openFirewall = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Open the web UI port in the host firewall.";
      };
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [ {
    environment.systemPackages = [ gb-grid-pkg pgPackage ];

    users.users.${dbUser} = {
      isSystemUser = true;
      group = dbUser;
      home = appHome;
      createHome = true;
    };
    users.groups.${dbUser} = {};

    systemd.tmpfiles.rules = [
      "d ${appHome} 0750 ${dbUser} ${dbUser} -"
    ];

    services.postgresql = {
      enable = true;
      package = pgPackage;
      enableTCPIP = true;
      settings = {
        shared_preload_libraries = "timescaledb";
        "timescaledb.telemetry_level" = "off";
      };
      ensureDatabases = [ dbName ];
      ensureUsers = [{
        name = dbUser;
        ensureDBOwnership = true;
        # `gb-grid migrate` creates the web API's web_anon role (see
        # migrations/post-apply.api.sql), which needs CREATEROLE.
        ensureClauses.createrole = lib.mkIf cfg.web.enable true;
      }];
    };

    services.grafana = {
      enable = true;
      settings = {
        server = {
          http_addr = "0.0.0.0";
          http_port = cfg.grafanaPort;
        };
        "auth.anonymous" = {
          enabled = true;
          org_role = "Editor";
        };
        security.allow_embedding = true;
        analytics = {
          reporting_enabled = false;
          check_for_updates = false;
        };
        security.secret_key = "SW2YcwTIb9zpOOhoPsMm";
      };
      provision = {
        enable = true;
        datasources.settings.datasources = [{
          name = "gb-grid";
          uid = "gbgrid";
          type = "postgres";
          access = "proxy";
          url = "127.0.0.1:5432";
          user = dbUser;
          isDefault = true;
          jsonData = {
            database = dbName;
            sslmode = "disable";
            postgresVersion = 1600;
            timescaledb = true;
          };
        }];
        dashboards.settings.providers = [{
          name = "gb-grid";
          orgId = 1;
          folder = "";
          type = "file";
          disableDeletion = false;
          updateIntervalSeconds = 30;
          options.path = dashboardsDir;
        }];
      };
    };

    networking.firewall.allowedTCPPorts =
      lib.optional cfg.openGrafanaFirewall cfg.grafanaPort;

    systemd.services.gb-grid = {
      description = "GB grid BMRS ingester";
      after = [ "network-online.target" "postgresql.service" ];
      requires = [ "postgresql.service" ];
      wants = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];

      environment.GB_GRID_DATABASE_URL = "postgresql:///${dbName}";

      serviceConfig = {
        User = dbUser;
        Group = dbUser;
        WorkingDirectory = appHome;
        ExecStartPre = [ "${timescaledbUpdate}" "${migrateBin} migrate" ];
        ExecStart = "${ingestBin} run";
        # The TimescaleDB hypertable conversion (migration 0009) rewrites the
        # large tables under migrate_data and takes minutes; without this the
        # default 90s start timeout would kill it mid-conversion.
        TimeoutStartSec = "1800";
        Restart = "on-failure";
        RestartSec = "30s";
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ReadWritePaths = [ appHome ];
      };
    };

    systemd.services.gb-grid-constraints = {
      description = "GB grid — refresh NESO day-ahead constraint flows";
      after = [ "network-online.target" "postgresql.service" ];
      requires = [ "postgresql.service" ];
      wants = [ "network-online.target" ];

      environment.GB_GRID_DATABASE_URL = "postgresql:///${dbName}";

      serviceConfig = {
        Type = "oneshot";
        User = dbUser;
        Group = dbUser;
        WorkingDirectory = appHome;
        ExecStart = "${ingestBin} refresh-constraints";
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ReadWritePaths = [ appHome ];
      };
    };

    systemd.timers.gb-grid-constraints = {
      description = "Daily refresh of NESO day-ahead constraint flows";
      wantedBy = [ "timers.target" ];
      timerConfig = {
        # Run at 08:00 UTC on weekdays — data is published on weekday mornings.
        OnCalendar = "Mon-Fri 08:00 UTC";
        # Also fire once on boot if the last run was missed (e.g. weekend).
        Persistent = true;
      };
    };
  }

  (lib.mkIf cfg.web.enable {
    assertions = [{
      assertion = gb-grid-web-pkg != null;
      message = "services.gb-grid.web.enable needs the web UI package: pass packages.gb-grid-web as specialArgs.gb-grid-web-pkg.";
    }];

    # PostgREST logs in as `postgrest` over the Unix socket (peer auth; the
    # nixpkgs module runs it as that DynamicUser) and runs each request as
    # web_anon. `gb-grid migrate` creates web_anon, grants it to `postgrest`,
    # and (re)builds the `api` schema, then notifies PostgREST to reload.
    services.postgresql.ensureUsers = [{ name = "postgrest"; }];

    services.postgrest = {
      enable = true;
      settings = {
        db-uri = {
          host = "/run/postgresql";
          dbname = dbName;
          user = "postgrest";
        };
        db-schemas = "api";
        db-anon-role = "web_anon";
        # Listens on its default Unix socket (/run/postgrest/postgrest.sock),
        # reachable only through nginx.
      };
    };
    systemd.services.postgrest = {
      after = [ "gb-grid.service" ];
      wants = [ "gb-grid.service" ];
    };

    services.nginx = {
      enable = true;
      recommendedGzipSettings = true;
      recommendedProxySettings = true;
      virtualHosts.gb-grid-web = {
        listen = [{ addr = "0.0.0.0"; port = cfg.web.port; }];
        root = gb-grid-web-pkg;
        # The build ships .gz siblings for every asset.
        extraConfig = "gzip_static on;";
        locations."/api/".proxyPass = "http://unix:/run/postgrest/postgrest.sock:/";
        locations."/_app/immutable/".extraConfig = ''
          add_header Cache-Control "public, max-age=31536000, immutable";
        '';
        # Client-side routes all load the single-page app.
        locations."/".tryFiles = "$uri /200.html";
      };
    };

    networking.firewall.allowedTCPPorts = lib.optional cfg.web.openFirewall cfg.web.port;
  })
  ]);
}
