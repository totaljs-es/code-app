// Total.js Code for Windows and Linux: the Total.js Code instances of a person, each in a window of its own, with the local sync of
// files that Code offers in its desktop app. The brother of TotalCode for Mac (TotalDesktop) and the successor of
// totaljs/code-electron, without Node.js in the pages: Code gets only the bridge of src/bridge.js, for its own project.
//
// Windows: "main" is the list of instances (src/index.html); "p-<id>" is an instance, "p-<id>-<n>" a window it opened.

mod bridge;
mod projects;

use projects::Project;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::ipc::CapabilityBuilder;
use tauri::webview::{DownloadEvent, WebviewWindowBuilder};
use tauri::{AppHandle, Manager, Url, WebviewUrl, Webview};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_opener::OpenerExt;

const MAIN: &str = "main";

struct State {
	file: PathBuf,
	projects: Mutex<Vec<Project>>,
	counter: AtomicU64,
	downloads: Mutex<HashMap<String, PathBuf>>,
}

impl State {
	fn find(&self, id: &str) -> Option<Project> {
		self.projects.lock().unwrap().iter().find(|p| p.id == id).cloned()
	}

	fn store(&self, projects: Vec<Project>) -> Result<(), String> {
		projects::save(&self.file, &projects).map_err(|e| e.to_string())?;
		*self.projects.lock().unwrap() = projects;
		Ok(())
	}
}

fn new_id() -> String {
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
	format!("{:x}{:04x}", now.as_millis(), now.subsec_nanos() & 0xffff)
}

// The project of a window: "p-<id>" or "p-<id>-<n>".
fn project_of_label(label: &str) -> Option<&str> {
	let rest = label.strip_prefix("p-")?;
	Some(rest.split('-').next().unwrap_or(rest))
}

fn outside(app: &AppHandle, url: &Url) {
	if matches!(url.scheme(), "http" | "https" | "mailto") {
		let _ = app.opener().open_url(url.as_str(), None::<&str>);
	}
}

// ---- The list (the page of the app)

#[derive(Deserialize)]
struct ProjectInput {
	id: Option<String>,
	name: String,
	url: String,
	folder: Option<String>,
}

#[tauri::command]
fn projects_list(state: tauri::State<'_, State>) -> Vec<Project> {
	state.projects.lock().unwrap().clone()
}

#[tauri::command]
fn project_save(state: tauri::State<'_, State>, project: ProjectInput) -> Result<Project, String> {
	let name = project.name.trim().chars().take(60).collect::<String>();
	if name.is_empty() {
		return Err("name".into());
	}
	let url = projects::normalize(&project.url).map_err(str::to_string)?;
	let folder = project.folder.map(|f| f.trim().to_string()).filter(|f| !f.is_empty());
	if folder.as_ref().is_some_and(|folder| !PathBuf::from(folder).is_absolute()) {
		return Err("folder".into());
	}
	let mut list = state.projects.lock().unwrap().clone();
	let saved = match project.id.as_deref().and_then(|id| list.iter_mut().find(|p| p.id == id)) {
		Some(existing) => {
			existing.name = name;
			existing.url = url.to_string();
			existing.folder = folder;
			existing.clone()
		}
		None => {
			let created = Project { id: new_id(), name, url: url.to_string(), folder };
			list.push(created.clone());
			created
		}
	};
	state.store(list)?;
	Ok(saved)
}

#[tauri::command]
fn project_remove(app: AppHandle, state: tauri::State<'_, State>, id: String) -> Result<(), String> {
	let list: Vec<Project> = state.projects.lock().unwrap().iter().filter(|p| p.id != id).cloned().collect();
	state.store(list)?;
	for (label, window) in app.webview_windows() {
		if project_of_label(&label) == Some(id.as_str()) {
			let _ = window.close();
		}
	}
	Ok(())
}

#[tauri::command]
fn projects_order(state: tauri::State<'_, State>, ids: Vec<String>) -> Result<(), String> {
	let mut list = state.projects.lock().unwrap().clone();
	list.sort_by_key(|p| ids.iter().position(|id| *id == p.id).unwrap_or(usize::MAX));
	state.store(list)
}

#[derive(Serialize)]
struct Ping {
	online: bool,
	version: Option<String>,
}

// Is the instance there, and which version of Code is it (GET /ping/, as the Electron app asked)?
#[tauri::command]
async fn project_ping(state: tauri::State<'_, State>, id: String) -> Result<Ping, String> {
	let project = state.find(&id).ok_or("project")?;
	let url = project.origin().ok_or("project")?.join("ping/").map_err(|e| e.to_string())?;
	let client = reqwest::Client::builder().timeout(Duration::from_secs(8)).build().map_err(|e| e.to_string())?;
	let Ok(response) = client.get(url).send().await else {
		return Ok(Ping { online: false, version: None });
	};
	if !response.status().is_success() {
		return Ok(Ping { online: false, version: None });
	}
	let version = response.json::<serde_json::Value>().await.ok().and_then(|v| match v.get("version") {
		Some(serde_json::Value::String(s)) => Some(s.clone()),
		Some(serde_json::Value::Number(n)) => Some(n.to_string()),
		_ => None,
	});
	Ok(Ping { online: true, version })
}

#[tauri::command]
async fn choose_folder(app: AppHandle) -> Option<String> {
	let picked = app.dialog().file().set_title("Folder for the local copy of the files").blocking_pick_folder()?;
	picked.into_path().ok().map(|p| p.to_string_lossy().to_string())
}

// Adds the instances of an export (Electron's, or this app's) that are not in the list yet.
#[tauri::command]
fn electron_import(state: tauri::State<'_, State>, text: String) -> Result<usize, String> {
	let places = projects::import(&text).map_err(str::to_string)?;
	let mut list = state.projects.lock().unwrap().clone();
	let mut added = 0;
	for (name, url, folder) in places {
		let Ok(url) = projects::normalize(&url) else { continue };
		if list.iter().any(|p| p.url == url.as_str()) {
			continue;
		}
		list.push(Project { id: new_id() + &added.to_string(), name, url: url.to_string(), folder });
		added += 1;
	}
	state.store(list)?;
	Ok(added)
}

#[tauri::command]
fn electron_export(state: tauri::State<'_, State>) -> String {
	projects::export(&state.projects.lock().unwrap())
}

#[tauri::command]
fn project_open(app: AppHandle, state: tauri::State<'_, State>, id: String) -> Result<(), String> {
	let project = state.find(&id).ok_or("project")?;
	let label = format!("p-{}", project.id);
	if let Some(window) = app.get_webview_window(&label) {
		let _ = window.unminimize();
		return window.set_focus().map_err(|e| e.to_string());
	}
	let url = project.origin().ok_or("project")?;
	open_window(&app, &project, label, url).map(|_| ())
}

// The pages of an instance may call the bridge_* commands, from its address and in its windows only.
fn grant(app: &AppHandle, project: &Project, origin: &Url) {
	let origin = origin.origin().ascii_serialization();
	let capability = CapabilityBuilder::new(format!("project-{}-{}", project.id, origin.replace(|c: char| !c.is_ascii_alphanumeric(), "-")))
		.remote(format!("{origin}/*"))
		.windows([format!("p-{}", project.id), format!("p-{}-*", project.id)])
		.permission("allow-bridge-write")
		.permission("allow-bridge-rename")
		.permission("allow-bridge-remove")
		.permission("allow-bridge-stat")
		.permission("allow-bridge-readdir")
		.permission("allow-bridge-download");
	let _ = app.add_capability(capability);
}

fn open_window(app: &AppHandle, project: &Project, label: String, url: Url) -> Result<tauri::WebviewWindow, String> {
	let origin = project.origin().ok_or("project")?;
	grant(app, project, &origin);
	let script = include_str!("../../src/bridge.js")
		.replace("__ORIGIN__", &origin.origin().ascii_serialization())
		.replace("__ROOT__", if project.folder().is_some() { bridge::ROOT } else { "" });

	let navigating = origin.clone();
	let opener = app.clone();
	let windows = app.clone();
	let opening = origin.clone();
	let owner = project.clone();
	let downloads = app.clone();
	let folder = project.folder();
	let name = project.name.clone();

	WebviewWindowBuilder::new(app, label, WebviewUrl::External(url))
		.title(format!("{} · Total.js Code", project.name))
		.inner_size(1280.0, 820.0)
		.min_inner_size(640.0, 420.0)
		.initialization_script(script)
		// Inside the window: the instance. Anything else opens in the system browser.
		.on_navigation(move |url| {
			if projects::same_origin(&navigating, url) || matches!(url.scheme(), "about" | "data" | "blob") {
				return true;
			}
			outside(&opener, url);
			false
		})
		// A window the instance opens (a preview of a file) is another window of the project, with the same sign-in.
		.on_new_window(move |url, _features| {
			if projects::same_origin(&opening, &url) {
				let state = windows.state::<State>();
				let n = state.counter.fetch_add(1, Ordering::Relaxed);
				let _ = open_window(&windows, &owner, format!("p-{}-{}", owner.id, n), url);
			} else {
				outside(&windows, &url);
			}
			tauri::webview::NewWindowResponse::Deny
		})
		// "Download file" and the bundles: into the folder of the project (bundles/ for a bundle), or Downloads.
		.on_download(move |_webview, event| {
			let state = downloads.state::<State>();
			match event {
				DownloadEvent::Requested { url, destination } => {
					let suggested = destination.file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty());
					let bundle = url.path().contains("/bundle/") || suggested.as_deref().is_some_and(|n| n.ends_with(".bundle"));
					let file = if bundle {
						format!("{}.bundle", name.chars().map(|c| if c.is_alphanumeric() || "-_ ".contains(c) { c } else { '-' }).collect::<String>())
					} else {
						suggested
							.or_else(|| url.path_segments().and_then(|mut s| s.next_back().map(str::to_string)).filter(|s| !s.is_empty()))
							.unwrap_or_else(|| "download".into())
					};
					let dir = match &folder {
						Some(folder) if bundle => folder.join("bundles"),
						Some(folder) => folder.clone(),
						None => downloads.path().download_dir().unwrap_or_else(|_| std::env::temp_dir()),
					};
					let _ = std::fs::create_dir_all(&dir);
					*destination = dir.join(file);
					state.downloads.lock().unwrap().insert(url.to_string(), destination.clone());
				}
				DownloadEvent::Finished { url, path, success } => {
					let saved = path.or_else(|| state.downloads.lock().unwrap().remove(url.as_str()));
					if let (true, Some(saved)) = (success, saved) {
						let reveal = downloads.clone();
						let shown = saved.clone();
						downloads
							.dialog()
							.message(format!("The file was saved to {}.", saved.display()))
							.title("Download saved")
							.buttons(MessageDialogButtons::OkCancelCustom("Show in folder".into(), "OK".into()))
							.show(move |show| {
								if show {
									let _ = reveal.opener().reveal_item_in_dir(&shown);
								}
							});
					}
				}
				_ => {}
			}
			true
		})
		.build()
		.map_err(|e| e.to_string())
}

// ---- The bridge (the pages of an instance; see src/bridge.js and bridge.rs)

// The folder of the project of the page that calls, once its address is the one of the project.
fn folder_for(webview: &Webview, state: &State) -> Result<(Project, PathBuf), String> {
	let id = project_of_label(webview.label()).ok_or("project")?;
	let project = state.find(id).ok_or("project")?;
	let (Some(origin), Ok(page)) = (project.origin(), webview.url()) else {
		return Err("project".into());
	};
	if !projects::same_origin(&origin, &page) {
		return Err("project".into());
	}
	let folder = project.folder().ok_or("You don't have defined repository path.")?;
	Ok((project, folder))
}

#[tauri::command]
fn bridge_write(webview: Webview, state: tauri::State<'_, State>, path: String, data: String) -> Result<(), String> {
	let (_, folder) = folder_for(&webview, &state)?;
	bridge::write(&folder, &path, &data)
}

#[tauri::command]
fn bridge_rename(webview: Webview, state: tauri::State<'_, State>, from: String, to: String) -> Result<(), String> {
	let (_, folder) = folder_for(&webview, &state)?;
	bridge::rename(&folder, &from, &to)
}

#[tauri::command]
fn bridge_remove(webview: Webview, state: tauri::State<'_, State>, path: String) -> Result<(), String> {
	let (_, folder) = folder_for(&webview, &state)?;
	bridge::remove(&folder, &path)
}

#[tauri::command]
fn bridge_stat(webview: Webview, state: tauri::State<'_, State>, path: String) -> Result<Option<bridge::Stat>, String> {
	let (_, folder) = folder_for(&webview, &state)?;
	Ok(bridge::stat(&folder, &path))
}

#[tauri::command]
fn bridge_readdir(webview: Webview, state: tauri::State<'_, State>, path: String) -> Result<Vec<bridge::Entry>, String> {
	let (_, folder) = folder_for(&webview, &state)?;
	bridge::readdir(&folder, &path)
}

// "Download last changes / source-code locally": a file of the instance into the folder. Only from the instance itself: the
// cookie of the session that Code adds must not reach any other server.
#[tauri::command]
async fn bridge_download(
	webview: Webview,
	state: tauri::State<'_, State>,
	href: String,
	headers: HashMap<String, String>,
	path: String,
) -> Result<u16, String> {
	let (project, folder) = folder_for(&webview, &state)?;
	let url = Url::parse(&href).map_err(|e| e.to_string())?;
	if !project.origin().is_some_and(|origin| projects::same_origin(&origin, &url)) {
		return Err("outside".into());
	}
	let file = bridge::resolve(&folder, &path)?;
	let client = reqwest::Client::builder().timeout(Duration::from_secs(120)).build().map_err(|e| e.to_string())?;
	let mut request = client.get(url);
	for (key, value) in headers {
		if matches!(key.to_ascii_lowercase().as_str(), "cookie" | "user-agent" | "authorization" | "accept") {
			request = request.header(key, value);
		}
	}
	let response = request.send().await.map_err(|e| e.to_string())?;
	let status = response.status().as_u16();
	if response.status().is_success() {
		let bytes = response.bytes().await.map_err(|e| e.to_string())?;
		if let Some(dir) = file.parent() {
			std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
		}
		std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
	}
	Ok(status)
}

// ---- The app

fn show_list(app: &AppHandle) {
	if let Some(window) = app.get_webview_window(MAIN) {
		let _ = window.unminimize();
		let _ = window.show();
		let _ = window.set_focus();
	} else {
		let _ = list_window(app);
	}
}

fn list_window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
	WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
		.title("Total.js Code")
		.inner_size(920.0, 680.0)
		.min_inner_size(420.0, 420.0)
		.build()
}

fn menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
	use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
	let main = Submenu::with_items(
		app,
		"Total.js Code",
		true,
		&[
			&MenuItem::with_id(app, "list", "Code instances", true, Some("CmdOrCtrl+Shift+L"))?,
			&PredefinedMenuItem::separator(app)?,
			&PredefinedMenuItem::quit(app, None)?,
		],
	)?;
	let edit = Submenu::with_items(
		app,
		"Edit",
		true,
		&[
			&PredefinedMenuItem::undo(app, None)?,
			&PredefinedMenuItem::redo(app, None)?,
			&PredefinedMenuItem::separator(app)?,
			&PredefinedMenuItem::cut(app, None)?,
			&PredefinedMenuItem::copy(app, None)?,
			&PredefinedMenuItem::paste(app, None)?,
			&PredefinedMenuItem::select_all(app, None)?,
		],
	)?;
	let view = Submenu::with_items(
		app,
		"View",
		true,
		&[
			&MenuItem::with_id(app, "reload", "Reload", true, Some("CmdOrCtrl+R"))?,
			&MenuItem::with_id(app, "devtools", "Developer tools", true, Some("F12"))?,
			&PredefinedMenuItem::separator(app)?,
			&PredefinedMenuItem::fullscreen(app, None)?,
		],
	)?;
	let help = Submenu::with_items(
		app,
		"Help",
		true,
		&[
			&MenuItem::with_id(app, "docs", "Total.js Code documentation", true, None::<&str>)?,
			&MenuItem::with_id(app, "source", "This app on GitHub", true, None::<&str>)?,
			&MenuItem::with_id(app, "privacy", "Privacy policy", true, None::<&str>)?,
		],
	)?;
	Menu::with_items(app, &[&main, &edit, &view, &help])
}

fn focused(app: &AppHandle) -> Option<tauri::WebviewWindow> {
	app.webview_windows().into_values().find(|w| w.is_focused().unwrap_or(false))
}

pub fn run() {
	tauri::Builder::default()
		.plugin(tauri_plugin_opener::init())
		.plugin(tauri_plugin_dialog::init())
		.plugin(tauri_plugin_window_state::Builder::new().build())
		.invoke_handler(tauri::generate_handler![
			projects_list,
			project_save,
			project_remove,
			projects_order,
			project_open,
			project_ping,
			choose_folder,
			electron_import,
			electron_export,
			bridge_write,
			bridge_rename,
			bridge_remove,
			bridge_stat,
			bridge_readdir,
			bridge_download
		])
		.setup(|app| {
			let file = app.path().app_config_dir()?.join("projects.json");
			let list = projects::load(&file);
			app.manage(State { file, projects: Mutex::new(list), counter: AtomicU64::new(1), downloads: Mutex::new(HashMap::new()) });
			app.set_menu(menu(app.handle())?)?;
			list_window(app.handle())?;
			Ok(())
		})
		.on_menu_event(|app, event| match event.id().as_ref() {
			"list" => show_list(app),
			"reload" => {
				if let Some(window) = focused(app) {
					let _ = window.eval("location.reload()");
				}
			}
			"devtools" => {
				if let Some(window) = focused(app) {
					window.open_devtools();
				}
			}
			"docs" => {
				let _ = app.opener().open_url("https://docs.totaljs.com/code/", None::<&str>);
			}
			"privacy" => {
				let _ = app.opener().open_url("https://totaldesktop.totaljs.es/totalcode/privacy/", None::<&str>);
			}
			"source" => {
				let _ = app.opener().open_url("https://github.com/totaljs-es/code-app", None::<&str>);
			}
			_ => {}
		})
		.run(tauri::generate_context!())
		.expect("error while running Total.js Code");
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_window_belongs_to_its_project() {
		assert_eq!(project_of_label("p-19a2b3c4d5e"), Some("19a2b3c4d5e"));
		assert_eq!(project_of_label("p-19a2b3c4d5e-3"), Some("19a2b3c4d5e"));
		assert_eq!(project_of_label("main"), None);
	}
}
