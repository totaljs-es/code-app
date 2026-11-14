// The list of Code instances: add, edit, remove, order, open, and import or export the list of the Electron app.
var invoke = window.__TAURI_INTERNALS__.invoke;
var list = document.getElementById('list'),
	empty = document.getElementById('empty'),
	search = document.getElementById('search'),
	form = document.getElementById('form'),
	transfer = document.getElementById('transfer'),
	confirmbox = document.getElementById('confirm'),
	toast = document.getElementById('toast');
var projects = [], states = {}, editing = null, removing = null;

var ERRORS = {
	name: 'Enter a name.',
	invalid: 'That is not a valid address.',
	https: 'Use https: a password must not travel in the clear across the internet. Plain http is accepted on this computer and on a private network only.',
	folder: 'Choose the folder again.',
	export: 'This is not an export of the Code app.'
};

var ICONS = {
	globe: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/></svg>',
	folder: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
	edit: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M4 20h4L19 9l-4-4L4 16z"/></svg>',
	remove: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/></svg>',
	up: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M6 15l6-6 6 6"/></svg>'
};

function escape(text) {
	return String(text == null ? '' : text).replace(/[&<>"']/g, function(c) { return '&#' + c.charCodeAt(0) + ';'; });
}

function message(text) {
	toast.textContent = text;
	toast.classList.add('show');
	clearTimeout(message.timer);
	message.timer = setTimeout(function() { toast.classList.remove('show'); }, 2400);
}

function render() {
	var q = search.value.trim().toLowerCase();
	var shown = projects.filter(function(p) { return !q || (p.name + ' ' + p.url).toLowerCase().indexOf(q) !== -1; });
	empty.hidden = projects.length > 0;
	search.hidden = projects.length < 2;
	list.innerHTML = shown.map(function(p, i) {
		var s = states[p.id];
		var state = !s ? '<span class="status"><i></i>Checking…</span>'
			: s.online ? '<span class="status online"><i></i>' + (s.version ? 'Version ' + escape(s.version) : 'Online') + '</span>'
				: '<span class="status offline"><i></i>Offline</span>';
		return '<li class="card" tabindex="0" data-id="' + escape(p.id) + '" aria-label="Open ' + escape(p.name) + '">' +
			'<h3>' + escape(p.name) + '</h3>' +
			'<div class="line">' + ICONS.globe + '<span>' + escape(p.url.replace(/\/$/, '')) + '</span></div>' +
			'<div class="line">' + ICONS.folder + '<span>' + (p.folder ? escape(p.folder) : 'No local folder') + '</span></div>' + state +
			'<div class="tools">' +
			(i && !q ? '<button type="button" data-do="up" aria-label="Move ' + escape(p.name) + ' up">' + ICONS.up + '</button>' : '') +
			'<button type="button" data-do="edit" aria-label="Edit ' + escape(p.name) + '">' + ICONS.edit + '</button>' +
			'<button type="button" class="remove" data-do="ask" aria-label="Remove ' + escape(p.name) + '">' + ICONS.remove + '</button></div></li>';
	}).join('');
}

async function load() {
	projects = await invoke('projects_list');
	render();
	projects.forEach(function(p) {
		invoke('project_ping', { id: p.id }).then(function(result) {
			states[p.id] = result;
			render();
		}, function() {});
	});
}

function open(id) {
	invoke('project_open', { id: id }).catch(function(err) { message(String(err)); });
}

function edit(project) {
	editing = project || null;
	form.querySelector('form').reset();
	document.getElementById('form-title').textContent = project ? 'Edit instance' : 'Add a Code instance';
	document.getElementById('f-name').value = project ? project.name : '';
	document.getElementById('f-url').value = project ? project.url : '';
	document.getElementById('f-folder').value = project && project.folder || '';
	document.getElementById('form-error').textContent = '';
	form.showModal();
	document.getElementById(project ? 'f-name' : 'f-url').focus();
}

async function save() {
	var error = document.getElementById('form-error');
	var url = document.getElementById('f-url').value.trim();
	var name = document.getElementById('f-name').value.trim();
	if (!name && url)
		name = url.replace(/^https?:\/\//, '').replace(/\/.*$/, '');
	try {
		var saved = await invoke('project_save', { project: { id: editing && editing.id, name: name, url: url, folder: document.getElementById('f-folder').value || null } });
		form.close();
		delete states[saved.id];
		await load();
		message(editing ? 'Saved' : 'Added');
	} catch (key) {
		error.textContent = ERRORS[key] || String(key);
	}
}

document.addEventListener('click', async function(e) {
	var button = e.target.closest('[data-do]');
	var card = e.target.closest('.card');
	var project = card && projects.find(function(p) { return p.id === card.dataset.id; });
	if (!button) {
		card && open(card.dataset.id);
		return;
	}
	e.preventDefault();
	switch (button.dataset.do) {
		case 'add':
			edit(null);
			break;
		case 'edit':
			edit(project);
			break;
		case 'save':
			save();
			break;
		case 'close':
			button.closest('dialog').close();
			break;
		case 'pick':
			var folder = await invoke('choose_folder');
			if (folder)
				document.getElementById('f-folder').value = folder;
			break;
		case 'unpick':
			document.getElementById('f-folder').value = '';
			break;
		case 'up':
			var ids = projects.map(function(p) { return p.id; });
			var index = ids.indexOf(project.id);
			ids.splice(index - 1, 0, ids.splice(index, 1)[0]);
			await invoke('projects_order', { ids: ids });
			await load();
			break;
		case 'ask':
			removing = project;
			document.getElementById('confirm-text').textContent = 'Remove “' + project.name + '” from the list? Its instance and the files of its folder are not touched.';
			confirmbox.showModal();
			break;
		case 'remove':
			await invoke('project_remove', { id: removing.id });
			confirmbox.close();
			await load();
			break;
		case 'transfer':
			document.getElementById('t-data').value = projects.length ? await invoke('electron_export') : '';
			document.getElementById('transfer-error').textContent = '';
			transfer.showModal();
			break;
		case 'copy':
			var data = document.getElementById('t-data');
			try {
				await navigator.clipboard.writeText(data.value);
			} catch (err) {
				data.select();
				document.execCommand('copy');
			}
			message('Copied');
			break;
		case 'import':
			try {
				var added = await invoke('electron_import', { text: document.getElementById('t-data').value });
				transfer.close();
				await load();
				message(added === 1 ? '1 instance added' : added + ' instances added');
			} catch (key) {
				document.getElementById('transfer-error').textContent = ERRORS[key] || String(key);
			}
			break;
	}
});

form.querySelector('form').addEventListener('submit', function(e) {
	e.preventDefault();
	save();
});

list.addEventListener('keydown', function(e) {
	var card = e.target.closest('.card');
	if (card && e.target === card && (e.key === 'Enter' || e.key === ' ')) {
		e.preventDefault();
		open(card.dataset.id);
	}
});

search.addEventListener('input', render);
search.addEventListener('keydown', function(e) {
	var first = list.querySelector('.card');
	if (e.key === 'Enter' && first)
		open(first.dataset.id);
});

window.addEventListener('focus', function() {
	invoke('projects_list').then(function(value) {
		if (JSON.stringify(value) !== JSON.stringify(projects))
			load();
	});
});

load();
