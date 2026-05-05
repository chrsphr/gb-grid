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
            if [ ! -d .venv ]; then
              uv sync --quiet || true
            fi
            export PATH="$PWD/.venv/bin:$PATH"
            echo "gb-grid devShell — run 'uv sync' then 'gb-grid --help'"
          '';
        };
      });
}
