{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";

    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";

    crane.url = "github:ipetkov/crane";

    systems.url = "github:nix-systems/default";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } (
      {
        inputs,
        lib,
        ...
      }:
      {
        systems = import inputs.systems;
        imports = [
          inputs.flake-parts.flakeModules.easyOverlay
          inputs.treefmt-nix.flakeModule
        ];
        perSystem =
          {
            config,
            self',
            pkgs,
            ...
          }:
          let
            craneLib = inputs.crane.mkLib pkgs;
            src = craneLib.cleanCargoSource (craneLib.path ./.);
            versionInfo = craneLib.crateNameFromCargoToml { cargoToml = ./Cargo.toml; };
            bareCommonArgs = {
              inherit (versionInfo) version;
              inherit src;
              nativeBuildInputs = with pkgs; [
                pkg-config
              ];
              buildInputs = with pkgs; [
                openssl
              ];
            };
            cargoArtifacts = craneLib.buildDepsOnly (
              bareCommonArgs
              // {
                pname = "tsukkomi";
              }
            );
            commonArgs = bareCommonArgs // {
              inherit cargoArtifacts;
            };
            mkPackage =
              name:
              craneLib.buildPackage (
                commonArgs
                // {
                  pname = name;
                  cargoExtraArgs = "-p ${name}";
                }
              );
          in
          {
            packages = {
              tsukkomi = mkPackage "tsukkomi";
              tsukkomi-telegram = mkPackage "tsukkomi-telegram";
              tsukkomi-matrix = mkPackage "tsukkomi-matrix";
            };
            overlayAttrs = {
              inherit (config.packages) tsukkomi;
            };
            checks = {
              inherit (self'.packages) tsukkomi tsukkomi-telegram tsukkomi-matrix;
              doc = craneLib.cargoDoc (
                commonArgs
                // {
                  cargoDocExtraArgs = "--workspace";
                }
              );
              fmt = craneLib.cargoFmt { inherit src; };
              nextest = craneLib.cargoNextest (
                commonArgs
                // {
                  cargoNextestExtraArgs = lib.escapeShellArgs [
                    "--workspace"
                    "--no-tests=warn"
                  ];
                }
              );
              clippy = craneLib.cargoClippy (
                commonArgs
                // {
                  cargoClippyExtraArgs = "--workspace --all-targets -- --deny warnings";
                }
              );
              shell = self'.devShells.default;
            };
            treefmt = {
              projectRootFile = "flake.nix";
              programs = {
                nixfmt.enable = true;
                rustfmt.enable = true;
              };
            };
            devShells.default = pkgs.mkShell {
              inputsFrom = builtins.attrValues self'.packages;
              packages = with pkgs; [
                rustup
                (python3.withPackages (p: with p; [ pyyaml ]))
              ];
            };
          };
      }
    );
}
