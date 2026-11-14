// The Code instances of the person: name, address and, optionally, the folder where Code keeps a local copy of the files.
// They are kept in projects.json, in the configuration folder of the app.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::Url;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Project {
	pub id: String,
	pub name: String,
	pub url: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub folder: Option<String>,
}

impl Project {
	pub fn origin(&self) -> Option<Url> {
		Url::parse(&self.url).ok()
	}

	pub fn folder(&self) -> Option<PathBuf> {
		self.folder.as_ref().filter(|f| !f.trim().is_empty()).map(PathBuf::from)
	}
}

pub fn load(file: &Path) -> Vec<Project> {
	std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save(file: &Path, projects: &[Project]) -> std::io::Result<()> {
	if let Some(dir) = file.parent() {
		std::fs::create_dir_all(dir)?;
	}
	let tmp = file.with_extension("json.tmp");
	std::fs::write(&tmp, serde_json::to_string_pretty(projects).unwrap_or_else(|_| "[]".into()))?;
	std::fs::rename(tmp, file)
}

// The address of a Code instance: https unless it is on this machine or a private network; the path is kept (an instance can
// live under a path), the query, the fragment and any user name are not.
pub fn normalize(input: &str) -> Result<Url, &'static str> {
	let text = input.trim();
	if text.is_empty() {
		return Err("invalid");
	}
	let text = if text.contains("://") { text.to_string() } else { format!("https://{text}") };
	let mut url = Url::parse(&text).map_err(|_| "invalid")?;
	let host = url.host_str().unwrap_or("").to_lowercase();
	if host.is_empty() {
		return Err("invalid");
	}
	match url.scheme() {
		"https" => {}
		"http" if local(&host) => {}
		"http" => return Err("https"),
		_ => return Err("invalid"),
	}
	let _ = url.set_username("");
	let _ = url.set_password(None);
	url.set_query(None);
	url.set_fragment(None);
	if !url.path().ends_with('/') {
		let path = format!("{}/", url.path());
		url.set_path(&path);
	}
	Ok(url)
}

pub fn local(host: &str) -> bool {
	let host = host.trim_start_matches('[').trim_end_matches(']');
	if host == "localhost" || host == "::1" || host.ends_with(".local") || host.ends_with(".localhost") || !host.contains('.') {
		return true;
	}
	let parts: Vec<u8> = host.split('.').filter_map(|p| p.parse().ok()).collect();
	parts.len() == 4
		&& (parts[0] == 127
			|| parts[0] == 10
			|| (parts[0] == 192 && parts[1] == 168)
			|| (parts[0] == 172 && (16..=31).contains(&parts[1])))
}

pub fn same_origin(a: &Url, b: &Url) -> bool {
	a.origin() == b.origin()
}

// ---- The export of the Electron app (and of this one): ENCRYPT(places, 'totaljscode') of jComponent.
// The JSON in UTF-8, each byte in hexadecimal, then "x" and HASH(hex + key) of jComponent in base 32.

pub const SECRET: &str = "totaljscode";

#[derive(Serialize, Deserialize)]
struct Place {
	name: String,
	url: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	path: Option<String>,
}

// HASH() of jComponent: the Java string hash over UTF-16 code units, as an unsigned 32-bit number.
fn hash(text: &str) -> u32 {
	let mut h: i32 = 0;
	for unit in text.encode_utf16() {
		h = (h << 5).wrapping_sub(h).wrapping_add(unit as i32);
	}
	h as u32
}

fn base32(mut value: u32) -> String {
	const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuv";
	if value == 0 {
		return "0".into();
	}
	let mut out = Vec::new();
	while value > 0 {
		out.push(DIGITS[(value % 32) as usize]);
		value /= 32;
	}
	out.reverse();
	String::from_utf8(out).unwrap()
}

pub fn encrypt(text: &str, key: &str) -> String {
	let hex: String = text.as_bytes().iter().map(|b| format!("{b:x}")).collect();
	let digest = base32(hash(&(hex.clone() + key)));
	format!("{hex}x{digest}")
}

pub fn decrypt(encoded: &str, key: &str) -> Option<String> {
	let index = encoded.rfind('x')?;
	let (hex, digest) = (&encoded[..index], &encoded[index + 1..]);
	if base32(hash(&(hex.to_string() + key))) != digest || hex.len() % 2 != 0 {
		return None;
	}
	let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect();
	String::from_utf8(bytes?).ok()
}

// An instance of an export: name, address and folder.
pub type Imported = (String, String, Option<String>);

// The instances of an export: the encrypted text of Electron, or plain JSON.
pub fn import(text: &str) -> Result<Vec<Imported>, &'static str> {
	let text = text.trim();
	let json = decrypt(text, SECRET).unwrap_or_else(|| text.to_string());
	let places: Vec<serde_json::Value> = serde_json::from_str(&json).map_err(|_| "export")?;
	Ok(places
		.into_iter()
		.filter_map(|p| {
			let name = p.get("name")?.as_str()?.trim().to_string();
			let url = p.get("url")?.as_str()?.trim().to_string();
			let path = p.get("path").and_then(|v| v.as_str()).map(str::to_string).filter(|s| !s.trim().is_empty());
			(!name.is_empty() && !url.is_empty()).then_some((name, url, path))
		})
		.collect())
}

pub fn export(projects: &[Project]) -> String {
	let places: Vec<Place> = projects.iter().map(|p| Place { name: p.name.clone(), url: p.url.clone(), path: p.folder.clone() }).collect();
	encrypt(&serde_json::to_string(&places).unwrap_or_else(|_| "[]".into()), SECRET)
}

#[cfg(test)]
mod tests {
	use super::*;

	// Made with ENCRYPT() of jComponent 19, the one of the Electron app.
	const ELECTRON: &str = "5b7b226e616d65223a2254726162616a6f20c3b1222c2275726c223a2268747470733a2f2f636f64652e6578616d706c652e636f6d222c2270617468223a222f686f6d652f676572612f6769742f227d5dx286ggm0";

	#[test]
	fn an_export_of_electron_is_read() {
		let places = import(ELECTRON).unwrap();
		assert_eq!(places, vec![("Trabajo ñ".into(), "https://code.example.com".into(), Some("/home/gera/git/".into()))]);
	}

	#[test]
	fn the_export_is_the_one_of_electron() {
		let json = r#"[{"name":"Trabajo ñ","url":"https://code.example.com","path":"/home/gera/git/"}]"#;
		assert_eq!(encrypt(json, SECRET), ELECTRON);
		assert_eq!(decrypt(ELECTRON, SECRET).as_deref(), Some(json));
	}

	#[test]
	fn a_changed_export_is_refused_and_plain_json_is_accepted() {
		assert!(decrypt(&ELECTRON.replace("x286ggm0", "x286ggm1"), SECRET).is_none());
		assert!(import("not an export").is_err());
		assert_eq!(import(r#"[{"name":"A","url":"https://a.test"},{"name":"","url":"x"}]"#).unwrap().len(), 1);
	}

	#[test]
	fn an_address_keeps_its_path() {
		assert_eq!(normalize("code.example.com").unwrap().as_str(), "https://code.example.com/");
		assert_eq!(normalize("https://u:p@code.example.com/team?x#y").unwrap().as_str(), "https://code.example.com/team/");
		assert_eq!(normalize("http://localhost:8000").unwrap().as_str(), "http://localhost:8000/");
		assert_eq!(normalize("http://code.example.com"), Err("https"));
		assert_eq!(normalize("ftp://x.test"), Err("invalid"));
	}
}
