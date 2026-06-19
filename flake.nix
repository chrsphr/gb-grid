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

        # Toolchain-only dev shell. The services (Postgres + Grafana) come from
        # docker-compose — the single source of truth shared with non-Nix users
        # — so this shell only provides the language toolchain and a psql client,
        # all pointed at the compose Postgres on 127.0.0.1:5433.
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            python313
            uv
            postgresql_16   # psql / pg_isready / pg_dump client tools
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

            # Connect to the docker-compose Postgres (see docker-compose.yml).
            export PGHOST=127.0.0.1
            export PGPORT=5433
            export PGUSER=gb_grid
            export PGPASSWORD=gbgrid
            export PGDATABASE=gb_grid
            export GB_GRID_DATABASE_URL="postgresql://gb_grid:gbgrid@127.0.0.1:5433/gb_grid"

            # Bring up the data services via the one orchestrator: docker compose.
            if command -v docker >/dev/null 2>&1; then
              echo "[gb-grid] starting db + grafana via docker compose"
              docker compose up -d db grafana >/dev/null 2>&1 || \
                echo "[gb-grid] 'docker compose up' failed — is the docker daemon running?"
              # Wait for Postgres to accept connections before migrating.
              for _ in $(seq 1 30); do
                pg_isready -q -h "$PGHOST" -p "$PGPORT" -U "$PGUSER" && break
                sleep 1
              done
            else
              echo "[gb-grid] docker not found — enable virtualisation.docker.enable,"
              echo "          or run 'docker compose up -d' yourself, then re-enter the shell."
            fi

            if [ ! -d .venv ]; then
              uv sync --quiet --all-extras || true
            fi
            export PATH="$PWD/.venv/bin:$PATH"

            # Apply any pending migrations (idempotent) against the compose DB.
            if [ -x .venv/bin/gb-grid ]; then
              gb-grid migrate >/dev/null 2>&1 || true
            fi

            echo "gb-grid devShell — db on $PGHOST:$PGPORT (docker compose), grafana on http://localhost:3000"
          '';
        };
      }) // {
        # NixOS module that bundles Postgres + Grafana + the ingester.
        # Consumers pass the app derivation via specialArgs as `gb-grid-pkg`.
        nixosModules.default = import ./nix/module.nix;
      };
}
