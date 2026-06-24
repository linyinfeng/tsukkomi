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
            bareCommonArgs = {
              inherit src;
              nativeBuildInputs = with pkgs; [
                pkg-config
              ];
              buildInputs = with pkgs; [
                openssl
              ];
            };
            cargoArtifacts = craneLib.buildDepsOnly bareCommonArgs;
            commonArgs = bareCommonArgs // {
              inherit cargoArtifacts;
            };
          in
          {
            packages = {
              default = config.packages.tsukkomi;
              tsukkomi = craneLib.buildPackage commonArgs;
            };
            overlayAttrs = {
              inherit (config.packages) tsukkomi;
            };
            checks = {
              inherit (self'.packages) tsukkomi;
              doc = craneLib.cargoDoc commonArgs;
              fmt = craneLib.cargoFmt { inherit src; };
              nextest = craneLib.cargoNextest (
                commonArgs
                // {
                  cargoNextestExtraArgs = lib.escapeShellArgs [ "--no-tests=warn" ];
                }
              );
              clippy = craneLib.cargoClippy (
                commonArgs
                // {
                  cargoClippyExtraArgs = "--all-targets -- --deny warnings";
                }
              );
            };
            treefmt = {
              projectRootFile = "flake.nix";
              programs = {
                nixfmt.enable = true;
                rustfmt.enable = true;
              };
            };
            devShells.default = pkgs.mkShell {
              inputsFrom = lib.attrValues self'.checks;
              packages = with pkgs; [
                rustup
              ];
            };
          };
      }
    );
}
