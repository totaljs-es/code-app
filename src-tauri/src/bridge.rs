// What the pages of Total.js Code ask of the Electron app for "Save locally", "Download last changes locally" and "Download
// source-code locally": files in the folder of the project. Code sees the folder as /local (getPath in src/bridge.js); every
// path is resolved inside the real folder, after "..", and the deepest part that exists is followed through its symlinks, so
// nothing can be read or written outside it. The same rule as CodeProjectBridge.swift of TotalCode for Mac.

use serde::Serialize;
use std::path::{Component, Path, PathBuf};

pub const ROOT: &str = "/local";

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stat {
	pub is_file: bool,
	pub is_directory: bool,
	pub size: u64,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
	pub name: String,
	pub is_file: bool,
	pub is_directory: bool,
}

fn canonical(path: &Path) -> Result<PathBuf, String> {
	std::fs::canonicalize(path).map_err(|_| "outside".to_string())
}

// The real path of a path of Code (/local/...), or an error when it would leave the folder.
pub fn resolve(root: &Path, path: &str) -> Result<PathBuf, String> {
	let rest = if path == ROOT {
		""
	} else if let Some(rest) = path.strip_prefix(ROOT).filter(|r| r.starts_with('/')) {
		rest
	} else {
		return Err("outside".into());
	};
	let mut parts: Vec<&str> = Vec::new();
	for part in rest.split(['/', '\\']) {
		match part {
			"" | "." => {}
			".." => {
				parts.pop().ok_or("outside")?;
			}
			name if name.contains(':') || name.contains('\0') => return Err("outside".into()),
			name => parts.push(name),
		}
	}
	std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
	let root = canonical(root)?;
	let mut candidate = root.clone();
	for part in &parts {
		candidate.push(part);
	}
	// The deepest part that exists, through its symlinks (a dangling symlink fails), and the rest as written.
	let mut existing = candidate.clone();
	let mut remainder: Vec<std::ffi::OsString> = Vec::new();
	while std::fs::symlink_metadata(&existing).is_err() {
		remainder.insert(0, existing.file_name().ok_or("outside")?.to_owned());
		existing = existing.parent().ok_or("outside")?.to_path_buf();
	}
	let mut real = canonical(&existing)?;
	for part in remainder {
		real.push(part);
	}
	if real.components().any(|c| matches!(c, Component::ParentDir)) || !real.starts_with(&root) {
		return Err("outside".into());
	}
	Ok(real)
}

pub fn write(root: &Path, path: &str, data: &str) -> Result<(), String> {
	let file = resolve(root, path)?;
	if let Some(dir) = file.parent() {
		std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
	}
	let tmp = file.with_file_name(format!(".{}.totalcode", file.file_name().and_then(|n| n.to_str()).unwrap_or("file")));
	std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
	std::fs::rename(&tmp, &file).map_err(|e| {
		let _ = std::fs::remove_file(&tmp);
		e.to_string()
	})
}

pub fn rename(root: &Path, from: &str, to: &str) -> Result<(), String> {
	let (from, to) = (resolve(root, from)?, resolve(root, to)?);
	if from == root_of(root)? || to == root_of(root)? {
		return Err("outside".into());
	}
	if let Some(dir) = to.parent() {
		std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
	}
	if let Ok(meta) = std::fs::symlink_metadata(&to) {
		if meta.is_dir() { std::fs::remove_dir_all(&to) } else { std::fs::remove_file(&to) }.map_err(|e| e.to_string())?;
	}
	std::fs::rename(from, to).map_err(|e| e.to_string())
}

fn root_of(root: &Path) -> Result<PathBuf, String> {
	canonical(root)
}

// The folder itself is never removed, only what is inside it.
pub fn remove(root: &Path, path: &str) -> Result<(), String> {
	let target = resolve(root, path)?;
	if target == root_of(root)? {
		return Err("outside".into());
	}
	match std::fs::symlink_metadata(&target) {
		Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&target).map_err(|e| e.to_string()),
		Ok(_) => std::fs::remove_file(&target).map_err(|e| e.to_string()),
		Err(_) => Ok(()),
	}
}

pub fn stat(root: &Path, path: &str) -> Option<Stat> {
	let meta = std::fs::metadata(resolve(root, path).ok()?).ok()?;
	Some(Stat { is_file: meta.is_file(), is_directory: meta.is_dir(), size: meta.len() })
}

pub fn readdir(root: &Path, path: &str) -> Result<Vec<Entry>, String> {
	let dir = resolve(root, path)?;
	let mut out = Vec::new();
	for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
		let entry = entry.map_err(|e| e.to_string())?;
		let name = entry.file_name().to_string_lossy().to_string();
		if name.starts_with('.') {
			continue;
		}
		let kind = entry.file_type().map_err(|e| e.to_string())?;
		out.push(Entry { name, is_file: !kind.is_dir(), is_directory: kind.is_dir() });
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_path_of_code_lands_in_the_folder() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("project");
		let real = canonical(dir.path()).unwrap().join("project");
		assert_eq!(resolve(&root, "/local").unwrap(), real);
		assert_eq!(resolve(&root, "/local/repo/src/index.js").unwrap(), real.join("repo/src/index.js"));
		assert_eq!(resolve(&root, "/local/repo/./a/../b.js").unwrap(), real.join("repo/b.js"));
	}

	#[test]
	fn nothing_leaves_the_folder() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("project");
		for path in ["/local/..", "/local/../x", "/local/a/../../x", "/etc/passwd", "local/x", "/localx/y", "/local\\..\\x", "/local/C:/x", "/local/a\\..\\..\\x"] {
			assert!(resolve(&root, path).is_err(), "{path}");
		}
	}

	#[cfg(unix)]
	#[test]
	fn a_symlink_out_of_the_folder_is_refused() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("project");
		let outside = dir.path().join("outside");
		std::fs::create_dir_all(&root).unwrap();
		std::fs::create_dir_all(&outside).unwrap();
		std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
		std::os::unix::fs::symlink(dir.path().join("nothing"), root.join("dangling")).unwrap();
		assert!(resolve(&root, "/local/link/secret").is_err());
		assert!(write(&root, "/local/link/secret", "x").is_err());
		assert!(resolve(&root, "/local/dangling").is_err());
		assert!(!outside.join("secret").exists());
	}

	#[test]
	fn files_are_written_renamed_listed_and_removed() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("project");
		write(&root, "/local/repo/src/a.js", "hola").unwrap();
		assert_eq!(std::fs::read_to_string(root.join("repo/src/a.js")).unwrap(), "hola");
		assert_eq!(stat(&root, "/local/repo/src/a.js"), Some(Stat { is_file: true, is_directory: false, size: 4 }));
		assert_eq!(stat(&root, "/local/repo/missing"), None);
		rename(&root, "/local/repo/src/a.js", "/local/repo/lib/b.js").unwrap();
		let names: Vec<String> = readdir(&root, "/local/repo").unwrap().into_iter().map(|e| e.name).collect();
		assert!(names.contains(&"lib".to_string()) && names.contains(&"src".to_string()));
		remove(&root, "/local/repo/lib").unwrap();
		assert!(!root.join("repo/lib").exists());
		assert!(remove(&root, "/local").is_err());
		assert!(root.exists());
	}
}
