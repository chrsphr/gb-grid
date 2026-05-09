{ config, lib, pkgs, gb-grid-pkg, ... }:

let
  cfg = config.services.gb-grid;
  dbName = "gb_grid";
  dbUser = "gb_grid";
  appHome = "/var/lib/gb-grid";
  dashboardsDir = ./../grafana/dashboards;
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
    environment.systemPackages = [ gb-grid-pkg pkgs.postgresql_16 ];

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
      package = pkgs.postgresql_16;
      enableTCPIP = true;
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
            timescaledb = false;
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
        Restart = "on-failure";
        RestartSec = "30s";
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ReadWritePaths = [ appHome ];
      };
    };
  };
}
