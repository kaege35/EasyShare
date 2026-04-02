const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ─── STATE ───────────────────────────────────────────────
let myName = '';
let myId = null;
let selectedUser = null;
let pendingTransfer = null; 
let logItems = {};
let logCount = 0;

// ─── INIT ─────────────────────────────────────────────────
document.addEventListener("DOMContentLoaded", () => {
  // Event Listeners
  document.getElementById('join-btn').addEventListener('click', joinNetwork);
  
  document.getElementById('name-input').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') joinNetwork();
  });

  const dropArea = document.getElementById('drop-area');
  
  // Tauri v2 drag-drop events (doğru event isimleri)
  listen('tauri://drag-enter', () => { if (selectedUser) dropArea.classList.add('dragging'); });
  listen('tauri://drag-over', () => { if (selectedUser) dropArea.classList.add('dragging'); });
  listen('tauri://drag-leave', () => dropArea.classList.remove('dragging'));

  // Sürükle-bırak: tauri://drag-drop (v2'de tauri://drop değil!)
  listen('tauri://drag-drop', async (e) => {
    dropArea.classList.remove('dragging');
    if (!selectedUser) { toast('Önce bir kişi seç', 'error'); return; }
    if (!selectedUser.ip) { toast('Ağ adresi yok', 'error'); return; }
    
    const paths = e.payload.paths;
    if (!paths || paths.length === 0) return;
    
    invoke('send_paths_directly', { peerIp: selectedUser.ip, paths });
  });

  document.getElementById('browse-btn').addEventListener('click', () => {
    if (!selectedUser) { toast('Önce bir kişi seç', 'error'); return; }
    if (!selectedUser.ip) { toast('Ağ adresi yok', 'error'); return; }
    invoke('open_file_dialog', { peerIp: selectedUser.ip });
  });

  // Yenile butonu
  document.getElementById('refresh-btn').addEventListener('click', async () => {
    const btn = document.getElementById('refresh-btn');
    btn.classList.add('spinning');
    try {
      await invoke('scan_network');
      toast('Ağ taranıyor...', 'info');
    } catch(e) {
      toast('Tarama hatası: ' + e, 'error');
    }
    setTimeout(() => btn.classList.remove('spinning'), 1500);
  });

  // Gelen onay istekleri
  listen('transfer-request', (event) => {
    const p = event.payload;
    showModal(p.id, p.total_files, p.total_size);
  });

  // Genel hata ve durum bildirimleri
  listen('transfer-event', (event) => {
    const msg = event.payload;
    if (msg.includes("ERİŞİM_REDDEDİLDİ")) {
      toast('Karşı taraf aktarımı reddetti', 'error');
    } else {
      toast(msg, 'error');
    }
  });

  // Gelen dosya ilerleme
  listen('transfer-progress', (event) => {
    const { id, pct, text, is_done } = event.payload;
    if (pct === 0 && text) {
      addLog(id, text, 'in', 'Başlıyor...');
    } else if (is_done) {
      updateLog(id, 'Tamamlandı', 'done', 100);
      toast(text || 'Transfer bitti!', 'success');
    } else {
      updateLog(id, `%${pct}`, '', pct);
    }
  });

  // Giden dosya ilerleme
  listen('transfer-out-progress', (event) => {
    const { id, pct, text, is_done } = event.payload;
    if (pct === 0 && text) {
      addLog(id, text, 'out', 'Gönderiliyor...');
    } else if (is_done) {
      updateLog(id, 'İletildi', 'success', 100);
      toast(text || 'Gönderim tamamlandı!', 'success');
    } else {
      updateLog(id, `%${pct}`, '', pct);
    }
  });

  // Ağ cihazları güncelleme
  listen('peers-updated', (event) => {
    updateUserList(event.payload);
  });

  // Otomatik güncelleme
  listen('update-available', (event) => {
    const version = event.payload;
    showUpdateBanner(version);
  });

  // Accept/Decline butonları
  const acceptBtn = document.getElementById('accept-btn');
  if(acceptBtn) acceptBtn.addEventListener('click', () => {
    document.getElementById('incoming-modal').classList.remove('visible');
    if (currentTransferId) invoke('respond_to_transfer', { id: currentTransferId, accept: true });
  });

  const declineBtn = document.getElementById('decline-btn');
  if(declineBtn) declineBtn.addEventListener('click', () => {
    document.getElementById('incoming-modal').classList.remove('visible');
    if (currentTransferId) invoke('respond_to_transfer', { id: currentTransferId, accept: false });
  });
});

function showUpdateBanner(version) {
  const existing = document.getElementById('update-banner');
  if (existing) existing.remove();

  const banner = document.createElement('div');
  banner.id = 'update-banner';
  banner.innerHTML = `
    <span>🚀 <strong>EasyShare v${version}</strong> mevcut!</span>
    <button onclick="doUpdate()">Hemen Güncelle</button>
    <button onclick="this.parentElement.remove()" style="background:transparent;border:none;color:inherit;cursor:pointer;margin-left:4px;font-size:16px;">✕</button>
  `;
  document.body.appendChild(banner);
}

window.doUpdate = async () => {
  const { relaunch } = window.__TAURI__.process;
  await invoke('install_update');
  await relaunch();
};

let currentTransferId = null;

function showModal(transferId, count, size) {
  currentTransferId = transferId;
  const overlay = document.getElementById('incoming-modal');
  document.getElementById('modal-file-name').textContent = `${count} adet dosya/klasör`;
  document.getElementById('modal-file-meta').textContent = formatSize(size);
  overlay.classList.add('visible');
}

// ─── LOGIN ────────────────────────────────────────────────
async function joinNetwork() {
  const name = document.getElementById('name-input').value.trim();
  if (!name) return;
  myName = name;
  
  try {
    // start_discovery artık self ID döndürüyor
    myId = await invoke('start_discovery', { name });
    
    document.getElementById('login-screen').style.display = 'none';
    document.getElementById('app').classList.add('visible');
    document.getElementById('header-name').textContent = myName;

    // WiFi SSID'yi al
    fetchWifiSSID();
  } catch(e) {
    toast('Ağa katılma hatası: ' + e, 'error');
  }
}

async function fetchWifiSSID() {
  try {
    const ssid = await invoke('get_wifi_ssid');
    const el = document.getElementById('wifi-name');
    if (ssid && el) {
      el.textContent = ssid;
    }
  } catch(e) {
    console.log('WiFi SSID alınamadı:', e);
    // Varsayılan metin kalır
  }
}

// ─── KULLANICI LİSTESİ ───────────────────────────────────
function updateUserList(users) {
  const list = document.getElementById('user-list');
  const count = document.getElementById('online-count');
  
  // ID bazlı filtreleme (isim yerine)
  const otherUsers = users.filter(u => u.id !== myId);
  count.textContent = otherUsers.length;
  list.innerHTML = '';
  
  users.forEach(u => {
    const isSelf = u.id === myId;
    const isSelected = selectedUser && selectedUser.id === u.id;
    const el = document.createElement('div');
    el.className = `user-item${isSelf ? ' self' : ''}${isSelected ? ' selected' : ''}`;
    
    const avatar = u.name.slice(0, 2).toUpperCase();

    el.innerHTML = `
      <div class="avatar">${avatar}</div>
      <div class="user-info">
        <div class="user-name">${u.name}${isSelf ? ' (sen)' : ''}</div>
        <div class="user-status">● çevrimiçi</div>
      </div>
      ${!isSelf ? '<div class="send-badge">GÖNDER →</div>' : ''}
    `;
    
    if (!isSelf) el.onclick = () => selectUser(u);
    list.appendChild(el);
  });

  // Seçili kullanıcı ağdan ayrıldıysa
  if (selectedUser && !users.find(u => u.id === selectedUser.id)) {
    const oldName = selectedUser.name;
    selectedUser = null;
    showDropUI(false);
    toast(`${oldName} ağdan ayrıldı`, 'error');
  }
}

function selectUser(user) {
  selectedUser = user;
  document.getElementById('drop-target-name').textContent = user.name + ' cihazına gönder';
  showDropUI(true);
  document.querySelectorAll('.user-item').forEach(el => el.classList.remove('selected'));
  document.querySelectorAll('.user-item').forEach(el => {
    if (el.querySelector('.user-name')?.textContent.startsWith(user.name)) {
      el.classList.add('selected');
    }
  });
}

function showDropUI(show) {
  document.getElementById('no-target').style.display = show ? 'none' : 'flex';
  document.getElementById('drop-target-ui').style.display = show ? 'flex' : 'none';
}

// ─── LOG ─────────────────────────────────────────────────
function addLog(peerId, fileName, direction, statusText) {
  const list = document.getElementById('log-list');
  const id = `log-${peerId}-${Date.now()}`;
  logItems[peerId] = id;
  logCount++;
  document.getElementById('log-count').textContent = logCount;

  const dirIcon = direction === 'out' 
    ? '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="17 11 12 6 7 11"/><line x1="12" y1="18" x2="12" y2="6"/></svg>'
    : '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="7 13 12 18 17 13"/><line x1="12" y1="6" x2="12" y2="18"/></svg>';
  
  const dirClass = direction === 'out' ? 'log-dir-out' : 'log-dir-in';

  const el = document.createElement('div');
  el.className = 'log-item';
  el.id = id;
  el.innerHTML = `
    <div class="log-icon ${dirClass}">${dirIcon}</div>
    <div class="log-text"><strong>${fileName}</strong></div>
    <div class="log-progress"><div class="log-progress-fill" style="width:0%"></div></div>
    <div class="log-status">${statusText}</div>
  `;
  list.prepend(el);
  list.scrollTop = 0;
}

function updateLog(peerId, statusText, statusClass, pct) {
  const id = logItems[peerId];
  if (!id) return;
  const el = document.getElementById(id);
  if (!el) return;
  const status = el.querySelector('.log-status');
  const fill = el.querySelector('.log-progress-fill');
  if (status) { 
    status.textContent = statusText; 
    status.className = `log-status ${statusClass || ''}`; 
  }
  if (statusClass) el.classList.add(statusClass);
  if (fill && pct !== undefined) fill.style.width = pct + '%';
}

// ─── TOAST ───────────────────────────────────────────────
function toast(msg, type = 'info') {
  const c = document.getElementById('toast-container');
  const el = document.createElement('div');
  el.className = `toast ${type}`;
  el.textContent = msg;
  c.appendChild(el);
  setTimeout(() => el.remove(), 3500);
}

function formatSize(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
  return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GB';
}
