{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";

    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    crane.url = "github:ipetkov/crane";

    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";

    git-hooks-nix.url = "github:cachix/git-hooks.nix";
    git-hooks-nix.inputs.nixpkgs.follows = "nixpkgs";

    nix-github-actions.url = "github:nix-community/nix-github-actions";
    nix-github-actions.inputs.nixpkgs.follows = "nixpkgs";

  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } (
      {
        self,
        inputs,
        lib,
        ...
      }:
      {
        systems = [
          "x86_64-linux"
          "aarch64-linux"
        ];
        imports = [
          inputs.flake-parts.flakeModules.easyOverlay
          inputs.treefmt-nix.flakeModule
          inputs.git-hooks-nix.flakeModule
        ];
        perSystem =
          {
            self',
            pkgs,
            config,
            ...
          }:
          let
            inherit (inputs.nixpkgs) lib;
            inherit (lib) escapeShellArgs readFile;
            inherit ((fromTOML (readFile ./Cargo.toml)).workspace.package) version;

            craneLib = inputs.crane.mkLib pkgs;

            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                (craneLib.fileset.commonCargoSources ./.)
                (lib.fileset.fileFilter (f: f.hasExt "md") ./crates/tsukkomi/prompts)
              ];
            };

            commonArgs = {
              pname = "tsukkomi";
              inherit version src;
              strictDeps = true;
              nativeBuildInputs = with pkgs; [
                pkg-config
              ];
              buildInputs = with pkgs; [
                openssl
                sqlite
              ];
            };

            cargoArtifacts = craneLib.buildDepsOnly commonArgs;

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
              inherit (config.packages) tsukkomi tsukkomi-matrix tsukkomi-telegram;
            };

            checks = {
              inherit (self'.packages) tsukkomi tsukkomi-telegram tsukkomi-matrix;

              clippy = craneLib.cargoClippy (
                commonArgs
                // {
                  inherit cargoArtifacts;
                  cargoClippyExtraArgs = escapeShellArgs [
                    "--workspace"
                    "--all-targets"
                    "--"
                    "--deny"
                    "warnings"
                  ];
                }
              );

              doc = craneLib.cargoDoc (
                commonArgs
                // {
                  inherit cargoArtifacts;
                  cargoDocExtraArgs = escapeShellArgs [
                    "--workspace"
                  ];
                  env.RUSTFLAGS = escapeShellArgs [
                    "--deny"
                    "warnings"
                  ];
                }
              );

              nextest = craneLib.cargoNextest (
                commonArgs
                // {
                  inherit cargoArtifacts;
                  cargoNextestExtraArgs = escapeShellArgs [
                    "--workspace"
                    "--no-tests=warn"
                  ];
                }
              );

            };

            treefmt = {
              projectRootFile = "flake.nix";
              programs = {
                nixfmt.enable = true;
                deadnix.enable = true;
                rustfmt.enable = true;
                taplo.enable = true;
              };
            };

            pre-commit = {
              check.enable = false;
              settings = {
                hooks =
                  let
                    flakeShowArgs = [
                      "flake"
                      "show"
                    ];
                    flakeCheckArgs = [
                      "flake"
                      "check"
                    ];
                  in
                  {
                    flake-show = {
                      enable = true;
                      entry = "nix";
                      args = flakeShowArgs;
                      pass_filenames = false;
                    };
                    flake-check = {
                      enable = true;
                      entry = "nix";
                      args = flakeCheckArgs;
                      pass_filenames = false;
                    };
                    commitlint = {
                      enable = true;
                      entry = "commitlint --edit";
                      stages = [ "commit-msg" ];
                      package = pkgs.commitlint;
                    };
                  };
              };
            };

            devShells.default = craneLib.devShell {
              shellHook = ''
                ${config.pre-commit.shellHook}
              '';
              packages =
                with pkgs;
                [
                  cargo-audit
                  cargo-nextest

                  config.treefmt.build.wrapper
                ]
                ++ config.pre-commit.settings.enabledPackages;
            };
          };
        flake = {
          nixosModules.tsukkomi = ./nixos/tsukkomi.nix;
          githubActions = inputs.nix-github-actions.lib.mkGithubMatrix {
            checks = self.checks;
          };
        };
      }
    );
}
