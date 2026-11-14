// Runs in the window of a Code instance before its own scripts (initialization_script in src-tauri/src/lib.rs). Total.js Code
// turns on its desktop features ("Save locally", "Download last changes locally", "Download source-code locally", "Download
// file locally") when navigator.userAgent includes "TotalDesktop" (or "Electron") and window.require exists (views/index.html
// of totaljs/code). This gives it the window.require of the Electron app with the modules it uses, and does the work in the
// app (bridge_* commands), only inside the folder of the project, which Code sees as /local.
// The port of the bridge of TotalCode for Mac (CodeProjectBridge.swift).
// __ORIGIN__: the origin of the instance; __ROOT__: "/local" when the project has a folder, "" when not.
(function() {
	var ORIGIN = '__ORIGIN__';
	var ROOT = '__ROOT__';

	if (window.__totalCodeBridge || location.origin !== ORIGIN)
		return;

	var ua = navigator.userAgent;
	if (ua.indexOf('TotalDesktop') === -1) {
		try {
			Object.defineProperty(Navigator.prototype, 'userAgent', { configurable: true, get: function() { return ua + ' TotalDesktop'; } });
		} catch (e) {}
	}

	// Code sizes its editor with ENV('dockmargin'): 78 on a Mac, which makes room for the title bar it draws in a frameless
	// Electron window. This app keeps the title bar of the system and Code does not draw its own (getMeta), so 50 is right
	// everywhere, as TotalCode for Mac does.
	(function() {
		var real = null;
		function patched(key, value) {
			if (key === 'dockmargin' && value !== undefined)
				value = 50;
			return real ? real.call(this, key, value) : value;
		}
		try {
			Object.defineProperty(window, 'ENV', { configurable: true, enumerable: true, get: function() { return patched; }, set: function(fn) { real = fn; } });
		} catch (e) {}
	})();

	// A form of Code (common.form) shows in the properties panel, as in TotalCode for Mac.
	(function dockable() {
		if (!window.WATCH || !window.SETTER) {
			setTimeout(dockable, 250);
			return;
		}
		WATCH('common.form', function(path, value) {
			value && SETTER('dockable/show', 'prop');
		});
	})();

	function invoke(cmd, args) {
		var ipc = window.__TAURI_INTERNALS__;
		return ipc ? ipc.invoke(cmd, args || {}) : Promise.reject('no ipc');
	}

	function done(promise, callback, map) {
		promise.then(
			function(result) { callback && callback(null, map ? map(result) : result); },
			function(err) { callback && callback(new Error(String(err))); }
		);
	}

	var events = {};

	function makeStat(info) {
		return info ? {
			size: info.size || 0,
			isFile: function() { return !!info.isFile; },
			isDirectory: function() { return !!info.isDirectory; }
		} : null;
	}

	function normalizeSegments(parts) {
		var output = [];
		for (var i = 0; i < parts.length; i++) {
			var part = parts[i];
			if (!part || part === '.')
				continue;
			if (part === '..')
				output.pop();
			else
				output.push(part);
		}
		return output;
	}

	function normalizeOptions(options) {
		if (typeof(options) === 'string')
			return { href: options, headers: {} };
		var output = Object.assign({}, options || {});
		if (!output.href) {
			var origin = (output.protocol || 'https:') + '//' + (output.hostname || output.host || '');
			if (output.port)
				origin += ':' + output.port;
			output.href = origin + (output.path || output.pathname || '');
		}
		output.headers = output.headers || {};
		return output;
	}

	// http.get(url, res => res.pipe(fs.createWriteStream(file))): the app downloads the file into the folder.
	function transport() {
		return {
			get: function(options, callback) {
				var normalized = normalizeOptions(options);
				var response = {
					statusCode: 200,
					handlers: {},
					on: function(event, handler) {
						(this.handlers[event] || (this.handlers[event] = [])).push(handler);
						return this;
					},
					emit: function(event, value) {
						var list = this.handlers[event] || [];
						for (var i = 0; i < list.length; i++) {
							try {
								list[i](value);
							} catch (e) {}
						}
					},
					pipe: function(destination) {
						invoke('bridge_download', { href: normalized.href, headers: normalized.headers, path: destination && destination.__path || '' }).then(
							function(status) {
								response.statusCode = status;
								response.emit('end');
							},
							function(err) {
								response.emit('error', new Error(String(err)));
								response.emit('end');
							}
						);
						return destination;
					}
				};
				setTimeout(function() { callback && callback(response); }, 0);
				return { on: function() { return this; } };
			}
		};
	}

	var modules = {
		electron: {
			remote: null,
			ipcRenderer: {
				sendSync: function(channel, payload) {
					if (channel === 'getMeta')
						// A falsy version keeps the page from drawing the title bar of the frameless Electron window.
						return { version: 0 };
					if (channel === 'getPath') {
						var url = payload && payload.url;
						return ROOT && (!url || url === ORIGIN) ? ROOT : '';
					}
					return null;
				},
				send: function() {},
				on: function(channel, handler) {
					(events[channel] || (events[channel] = [])).push(handler);
				}
			}
		},
		fs: {
			writeFile: function(path, data, callback) {
				done(invoke('bridge_write', { path: path, data: String(data == null ? '' : data) }), callback && function(err) { callback(err); });
			},
			rename: function(from, to, callback) {
				done(invoke('bridge_rename', { from: from, to: to }), callback && function(err) { callback(err); });
			},
			unlink: function(path, callback) {
				done(invoke('bridge_remove', { path: path }), callback && function(err) { callback(err); });
			},
			rmdir: function(path, callback) {
				done(invoke('bridge_remove', { path: path }), callback && function(err) { callback(err); });
			},
			stat: function(path, callback) {
				invoke('bridge_stat', { path: path }).then(
					function(info) { callback && (info ? callback(null, makeStat(info)) : callback(new Error('ENOENT'))); },
					function(err) { callback && callback(new Error(String(err))); }
				);
			},
			// Only FUNC.mkdir of Code uses these two, before a writeFile that creates its folders by itself.
			statSync: function() {
				throw new Error('ENOENT');
			},
			mkdirSync: function() {},
			readdir: function(path, options, callback) {
				if (typeof(options) === 'function') {
					callback = options;
					options = {};
				}
				var types = !!(options && options.withFileTypes);
				done(invoke('bridge_readdir', { path: path }), callback, function(items) {
					return (items || []).map(function(item) {
						return types ? { name: item.name, isFile: function() { return item.isFile; }, isDirectory: function() { return item.isDirectory; } } : item.name;
					});
				});
			},
			createWriteStream: function(path) {
				return { __path: path };
			}
		},
		path: {
			join: function() {
				var parts = [];
				for (var i = 0; i < arguments.length; i++) {
					if (arguments[i] != null)
						parts.push(String(arguments[i]));
				}
				var absolute = parts.length && parts[0].charAt(0) === '/';
				return (absolute ? '/' : '') + normalizeSegments(parts.join('/').split('/')).join('/');
			},
			dirname: function(path) {
				if (!path)
					return '.';
				var clean = String(path).replace(/\/+$/, '');
				var index = clean.lastIndexOf('/');
				if (index <= 0)
					return index === 0 ? '/' : '.';
				return clean.substring(0, index);
			}
		},
		// "/" paths everywhere: the folder is /local whatever the system (FUNC.mkdir of Code reads the platform).
		os: {
			platform: function() { return 'linux'; },
			tmpdir: function() { return '/tmp'; }
		},
		url: {
			parse: function(value) {
				var parsed = new URL(value, location.href);
				return { href: parsed.href, protocol: parsed.protocol, hostname: parsed.hostname, host: parsed.host, port: parsed.port, pathname: parsed.pathname, path: parsed.pathname + parsed.search, search: parsed.search, hash: parsed.hash };
			}
		},
		http: transport(),
		https: transport()
	};

	window.__totalCodeBridge = { emit: function(channel, args) { (events[channel] || []).forEach(function(fn) { try { fn.apply(null, args || []); } catch (e) {} }); } };
	window.process = window.process || { env: {}, platform: 'linux' };
	window.require = function(name) {
		return modules[name];
	};
})();
