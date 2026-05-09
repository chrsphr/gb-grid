{
  description = "GB power grid energy database (BMRS Insights ingester)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        python = pkgs.python313;

        gb-grid = python.pkgs.buildPythonApplication {
          pname = "gb-grid";
          version = "0.1.0";
          pyproject = true;
          src = ./.;

          build-system = with python.pkgs; [ hatchling ];

          # nixpkgs ships psycopg2 (source build); the wheel asks for
          # psycopg2-binary. They're functionally identical at runtime.
          pythonRemoveDeps = [ "psycopg2-binary" ];

          dependencies = with python.pkgs; [
            httpx
            tenacity
            psycopg
            psycopg-pool
            psycopg2
            yoyo-migrations
            pydantic
            typer
            structlog
            python-dateutil
            pandas
          ];

          doCheck = false;
        };
      in {
        packages.default = gb-grid;
        packages.gb-grid = gb-grid;

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            python313
            uv
            postgresql_16
            grafana
            just
            ruff
            stdenv.cc.cc.lib   # libstdc++.so.6 for prebuilt wheels (pyzmq, etc.)
            zlib
          ];

          # uv installs prebuilt manylinux wheels that dlopen against glibc-style
          # paths. On NixOS we must point them at the Nix-provided libs.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.stdenv.cc.cc.lib
            pkgs.zlib
          ];

          shellHook = ''
            export UV_PYTHON=${pkgs.python313}/bin/python
            export PYTHONDONTWRITEBYTECODE=1

            # Ephemeral Postgres for development & tests.
            # Lives in $PWD/.postgres/, listens on a unix socket in the same dir.
            export PGDATA="$PWD/.postgres/data"
            export PGHOST="$PWD/.postgres"
            export PGPORT=5433
            export PGDATABASE=gb_grid
            export GB_GRID_DATABASE_URL="postgresql://localhost:$PGPORT/gb_grid?host=$PGHOST"

            mkdir -p "$PGHOST"
            if [ ! -d "$PGDATA" ]; then
              echo "[gb-grid] initdb in $PGDATA"
              initdb --username="$USER" --auth=trust --no-locale --encoding=UTF8 -D "$PGDATA" >/dev/null
              {
                echo "unix_socket_directories = '$PGHOST'"
                echo "listen_addresses = '127.0.0.1'"
                echo "port = $PGPORT"
              } >> "$PGDATA/postgresql.conf"
            fi

            if ! pg_ctl -D "$PGDATA" status >/dev/null 2>&1; then
              echo "[gb-grid] starting postgres on $PGHOST"
              pg_ctl -D "$PGDATA" -l "$PGHOST/postgres.log" -o "-k '$PGHOST'" start >/dev/null
              # Create the dev database if it doesn't exist
              if ! psql -h "$PGHOST" -lqt | cut -d'|' -f1 | grep -qw gb_grid; then
                createdb -h "$PGHOST" gb_grid
              fi
            fi

            if [ ! -d .venv ]; then
              uv sync --quiet --all-extras || true
            fi
            export PATH="$PWD/.venv/bin:$PATH"

            # Apply any pending migrations (idempotent).
            if [ -x .venv/bin/gb-grid ]; then
              gb-grid migrate >/dev/null 2>&1 || true
            fi

            # Ephemeral Grafana on :3000, provisioned from ./grafana/.
            export GF_PORT=3000
            export GF_PATHS_DATA="$PWD/.grafana/data"
            export GF_PATHS_LOGS="$PWD/.grafana/logs"
            export GF_PATHS_PLUGINS="$PWD/.grafana/plugins"
            export GF_PATHS_PROVISIONING="$PWD/.grafana/provisioning"
            export GB_GRID_DASHBOARDS_DIR="$PWD/grafana/dashboards"
            mkdir -p "$GF_PATHS_DATA" "$GF_PATHS_LOGS" "$GF_PATHS_PLUGINS" "$GF_PATHS_PROVISIONING"

            # Render provisioning templates with current env values (Grafana's own
            # $VAR substitution is unreliable across versions / unbraced names).
            ${pkgs.findutils}/bin/find "$PWD/grafana/provisioning" -type f | while read -r src; do
              dst="$GF_PATHS_PROVISIONING/''${src#$PWD/grafana/provisioning/}"
              mkdir -p "$(dirname "$dst")"
              ${pkgs.envsubst}/bin/envsubst < "$src" > "$dst"
            done

            cat > "$PWD/.grafana/grafana.ini" <<EOF
[server]
http_port = $GF_PORT
[paths]
data = $GF_PATHS_DATA
logs = $GF_PATHS_LOGS
plugins = $GF_PATHS_PLUGINS
provisioning = $GF_PATHS_PROVISIONING
[auth.anonymous]
enabled = true
org_role = Editor
[security]
admin_user = admin
admin_password = admin
allow_embedding = true
[analytics]
reporting_enabled = false
check_for_updates = false
EOF

            if ! kill -0 "$(cat "$PWD/.grafana/grafana.pid" 2>/dev/null)" 2>/dev/null; then
              echo "[gb-grid] starting grafana on http://localhost:$GF_PORT"
              ${pkgs.grafana}/bin/grafana server \
                --homepath ${pkgs.grafana}/share/grafana \
                --config "$PWD/.grafana/grafana.ini" \
                >"$GF_PATHS_LOGS/grafana.out" 2>&1 &
              echo $! > "$PWD/.grafana/grafana.pid"
            fi

            # Stop postgres + grafana when the shell exits.
            trap '
              pg_ctl -D "$PGDATA" stop -m fast >/dev/null 2>&1 || true
              [ -f "$PWD/.grafana/grafana.pid" ] && kill "$(cat "$PWD/.grafana/grafana.pid")" 2>/dev/null || true
              rm -f "$PWD/.grafana/grafana.pid"
            ' EXIT

            echo "gb-grid devShell — postgres on $PGHOST, db=gb_grid, grafana on :$GF_PORT"
          '';
        };
      });
}
