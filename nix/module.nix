{ config, lib, pkgs, gb-grid-pkg, ... }:

let
  cfg = config.services.gb-grid;
  dbName = "gb_grid";
  dbUser = "gb_grid";
  appHome = "/var/lib/gb-grid";
  dashboardsDir = ./../grafana/dashboards;

  # TimescaleDB is under the (unfree) TSL licence. Allow just that one package
  # via a scoped pkgs import so consumers don't need host-wide allowUnfree.
  tsPkgs = import pkgs.path {
    inherit (pkgs) system;
    config = pkgs.config // {
      allowUnfreePredicate = p: builtins.elem (lib.getName p) [ "timescaledb" ];
    };
  };
  pgPackage = tsPkgs.postgresql_16.withPackages (p: [ p.timescaledb ]);
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
  };

  config = lib.mkIf cfg.enable {
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
        ExecStartPre = "${gb-grid-pkg}/bin/gb-grid migrate";
        ExecStart = "${gb-grid-pkg}/bin/gb-grid run";
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
        ExecStart = "${gb-grid-pkg}/bin/gb-grid refresh-constraints";
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
  };
}
