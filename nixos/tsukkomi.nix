{
  pkgs,
  config,
  lib,
  ...
}:
let
  cfg = config.services.tsukkomi;
in
{
  options.services.tsukkomi = {
    deepseekApiKeyFile = lib.mkOption {
      type = lib.types.path;
      description = "Path to DeepSeek API key file";
    };

    xiaomiMimoApiKeyFile = lib.mkOption {
      type = lib.types.path;
      description = "Path to Xiaomi MiMo API key file";
    };

    extraArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "Extra arguments passed to both tsukkomi binaries (e.g. --debounce-duration 10s)";
    };

    matrix = {
      enable = lib.mkEnableOption "tsukkomi Matrix backend";

      package = lib.mkOption {
        type = lib.types.package;
        default = pkgs.tsukkomi-matrix;
        defaultText = lib.literalExpression "pkgs.tsukkomi-matrix";
        description = "tsukkomi-matrix package to use";
      };

      homeserver = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Matrix homeserver URL";
      };

      username = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Matrix username";
      };

      passwordFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to Matrix password file";
      };

      recoveryKeyFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to Matrix recovery key file (optional)";
      };

      rooms = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Matrix room IDs to monitor";
      };

      extraArgs = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Extra arguments specific to the Matrix backend";
      };
    };

    telegram = {
      enable = lib.mkEnableOption "tsukkomi Telegram backend";

      package = lib.mkOption {
        type = lib.types.package;
        default = pkgs.tsukkomi-telegram;
        defaultText = lib.literalExpression "pkgs.tsukkomi-telegram";
        description = "tsukkomi-telegram package to use";
      };

      tokenFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to Telegram bot token file";
      };

      chats = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Telegram chat IDs to monitor";
      };

      extraArgs = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Extra arguments specific to the Telegram backend";
      };
    };
  };

  config = lib.mkMerge [
    (lib.mkIf cfg.matrix.enable {
      assertions = [
        {
          assertion = cfg.matrix.homeserver != null;
          message = "tsukkomi: matrix.homeserver must be set when matrix backend is enabled";
        }
        {
          assertion = cfg.matrix.username != null;
          message = "tsukkomi: matrix.username must be set when matrix backend is enabled";
        }
        {
          assertion = cfg.matrix.passwordFile != null;
          message = "tsukkomi: matrix.passwordFile must be set when matrix backend is enabled";
        }
      ];

      systemd.services.tsukkomi-matrix = {
        description = "tsukkomi AI bot (Matrix)";
        after = [ "network-online.target" ];
        requires = [ "network-online.target" ];
        wantedBy = [ "multi-user.target" ];

        script = ''
          export MATRIX_PASSWORD=$(cat "$CREDENTIALS_DIRECTORY/matrix-password")
          export XIAOMI_MIMO_API_KEY=$(cat "$CREDENTIALS_DIRECTORY/xiaomi-mimo-api-key")
          export DEEPSEEK_API_KEY=$(cat "$CREDENTIALS_DIRECTORY/deepseek-api-key")
          ${lib.optionalString (cfg.matrix.recoveryKeyFile != null) ''
            export MATRIX_RECOVERY_KEY=$(cat "$CREDENTIALS_DIRECTORY/matrix-recovery-key")
          ''}
          exec "${cfg.matrix.package}/bin/tsukkomi-matrix" \
            --homeserver "${cfg.matrix.homeserver}" \
            --username "${cfg.matrix.username}" \
            --rooms "${lib.concatStringsSep "," cfg.matrix.rooms}" \
            ${lib.escapeShellArgs (cfg.extraArgs ++ cfg.matrix.extraArgs)}
        '';

        serviceConfig = {
          User = "tsukkomi";
          Group = "tsukkomi";
          StateDirectory = "tsukkomi";
          WorkingDirectory = "/var/lib/tsukkomi";
          LoadCredential = [
            "xiaomi-mimo-api-key:${cfg.xiaomiMimoApiKeyFile}"
            "deepseek-api-key:${cfg.deepseekApiKeyFile}"
            "matrix-password:${cfg.matrix.passwordFile}"
          ]
          ++ lib.optionals (cfg.matrix.recoveryKeyFile != null) [
            "matrix-recovery-key:${cfg.matrix.recoveryKeyFile}"
          ];
          Restart = "on-failure";
        };

        environment.RUST_LOG = "tsukkomi=info";
      };
    })

    (lib.mkIf cfg.telegram.enable {
      assertions = [
        {
          assertion = cfg.telegram.tokenFile != null;
          message = "tsukkomi: telegram.tokenFile must be set when telegram backend is enabled";
        }
      ];

      systemd.services.tsukkomi-telegram = {
        description = "tsukkomi AI bot (Telegram)";
        after = [ "network-online.target" ];
        requires = [ "network-online.target" ];
        wantedBy = [ "multi-user.target" ];

        script = ''
          export TELOXIDE_TOKEN=$(cat "$CREDENTIALS_DIRECTORY/telegram-token")
          export XIAOMI_MIMO_API_KEY=$(cat "$CREDENTIALS_DIRECTORY/xiaomi-mimo-api-key")
          export DEEPSEEK_API_KEY=$(cat "$CREDENTIALS_DIRECTORY/deepseek-api-key")
          exec "${cfg.telegram.package}/bin/tsukkomi-telegram" \
            --chats "${lib.concatStringsSep "," cfg.telegram.chats}" \
            ${lib.escapeShellArgs (cfg.extraArgs ++ cfg.telegram.extraArgs)}
        '';

        serviceConfig = {
          User = "tsukkomi";
          Group = "tsukkomi";
          StateDirectory = "tsukkomi";
          WorkingDirectory = "/var/lib/tsukkomi";
          LoadCredential = [
            "xiaomi-mimo-api-key:${cfg.xiaomiMimoApiKeyFile}"
            "deepseek-api-key:${cfg.deepseekApiKeyFile}"
            "telegram-token:${cfg.telegram.tokenFile}"
          ];
          Restart = "on-failure";
        };

        environment.RUST_LOG = "tsukkomi=info";
      };
    })

    (lib.mkIf (cfg.matrix.enable || cfg.telegram.enable) {
      users.users.tsukkomi = {
        isSystemUser = true;
        group = "tsukkomi";
      };
      users.groups.tsukkomi = { };
    })
  ];
}
