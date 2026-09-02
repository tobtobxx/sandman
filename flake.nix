{
	description = "An AI agent swarm that communicates through tasks";

	inputs = {
		nixpkgs.url = "github:nixos/nixpkgs/nixos-26.05";
		flake-utils.url = "github:numtide/flake-utils";
	};

	outputs = { self, nixpkgs, flake-utils }:
		flake-utils.lib.eachDefaultSystem (system:
			let
				pkgs = import nixpkgs { inherit system; };
			in {
				packages.default = pkgs.rustPlatform.buildRustPackage (finalAttrs: {
					pname = "sandman";
					version = "4.1.0";
					src = ./.;
					cargoHash = "sha256-tsa3qN3+nL44GUeWb3I3KF9q66SznBSiAsc8kGwm2P4=";
				});
			});
}
