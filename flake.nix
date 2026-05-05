{
  description = "GB power grid energy database (BMRS Insights ingester)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            python313
            uv
            duckdb
            just
            ruff
            sqlite-utils
          ];

          shellHook = ''
            export UV_PYTHON=${pkgs.python313}/bin/python
            export PYTHONDONTWRITEBYTECODE=1
            if [ ! -d .venv ]; then
              uv sync --quiet || true
            fi
            export PATH="$PWD/.venv/bin:$PATH"
            echo "gb-grid devShell — run 'uv sync' then 'gb-grid --help'"
          '';
        };
      });
}
