{ self }:

{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.ura;
  system = pkgs.stdenv.hostPlatform.system;

  mpvWithMpris = pkgs.mpv.override {
    scripts = [ pkgs.mpvScripts.mpris ];
  };
in
{
  options.services.ura = {
    enable = lib.mkEnableOption "ura audio node";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${system}.default;
      defaultText = lib.literalExpression "inputs.ura.packages.\${pkgs.stdenv.hostPlatform.system}.default";
      description = "The ura package to use.";
    };

    bind = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:8765";
      example = "0.0.0.0:8765";
      description = "Address and port on which ura accepts peer connections.";
    };

    logLevel = lib.mkOption {
      type = lib.types.str;
      default = "ura=info";
      description = "RUST_LOG value for the ura service.";
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [
      cfg.package
    ];

    systemd.user.services.ura = {
      Unit = {
        Description = "ura audio node";
        After = [ "network.target" ];
      };

      Service = {
        Type = "simple";

        ExecStart = lib.escapeShellArgs [
          "${cfg.package}/bin/ura"
          "daemon"
          "--bind"
          cfg.bind
        ];

        Environment = [
          "RUST_LOG=${cfg.logLevel}"
          "PATH=${
            lib.makeBinPath [
              mpvWithMpris
              pkgs.yt-dlp
            ]
          }"
        ];

        Restart = "on-failure";
        RestartSec = 2;
      };

      Install = {
        WantedBy = [ "default.target" ];
      };
    };
  };
}
