fn main() {
	// The commands of the app have permissions of their own (allow-<name>). The launcher (capabilities/launcher.json) gets
	// the project commands; the pages of a Code instance get only the bridge_* commands, at run time and for their own
	// project (src/lib.rs, grant).
	tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
		"projects_list",
		"project_save",
		"project_remove",
		"projects_order",
		"project_open",
		"project_ping",
		"choose_folder",
		"electron_import",
		"electron_export",
		"bridge_write",
		"bridge_rename",
		"bridge_remove",
		"bridge_stat",
		"bridge_readdir",
		"bridge_download",
	])))
	.expect("failed to run the build script");
}
