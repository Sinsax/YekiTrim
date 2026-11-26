// YekiTrim · LLM 配置交互方案参考稿 · 公共脚本
// 只做参考稿需要的三件事：主题切换（与 app 同一个 localStorage 键）、可折叠块、按钮假反馈。
(function () {
  var KEY = 'yekitrim.ui.theme'; // 与 webui/src/stores/theme.js 同键
  function read() {
    try { return localStorage.getItem(KEY) || ''; } catch (e) { return ''; }
  }
  function apply(v) {
    if (v === 'dark' || v === 'light') document.documentElement.setAttribute('data-theme', v);
    else document.documentElement.removeAttribute('data-theme');
    // 按钮上写的是「切过去会变成什么」：亮色模式下写"暗色"
    document.querySelectorAll('[data-theme-label]').forEach(function (el) {
      el.textContent = (document.documentElement.getAttribute('data-theme') === 'dark') ? '亮色' : '暗色';
    });
  }
  window.toggleTheme = function () {
    var cur = document.documentElement.getAttribute('data-theme') === 'dark' ? 'dark' : 'light';
    var next = cur === 'dark' ? 'light' : 'dark';
    try { localStorage.setItem(KEY, next); } catch (e) {}
    apply(next);
  };
  apply(read());

  // 通用折叠：<button data-fold="#id"> 切换 .fold-open（配合 .fold / .fold.open{display:block}）
  document.addEventListener('click', function (e) {
    var t = e.target.closest('[data-fold]');
    if (t) {
      var box = document.querySelector(t.getAttribute('data-fold'));
      if (!box) return;
      var open = box.classList.toggle('open');
      t.setAttribute('aria-expanded', open ? 'true' : 'false');
      var lbl = t.querySelector('[data-fold-label]');
      if (lbl) lbl.textContent = open ? (t.dataset.labelOpen || '收起') : (t.dataset.labelClosed || '展开');
      else if (t.dataset.labelClosed) t.textContent = open ? (t.dataset.labelOpen || '收起') : t.dataset.labelClosed;
      return;
    }
    // 假测试按钮：改成"测试中…"再回一个延迟
    var b = e.target.closest('[data-fake-test]');
    if (b) {
      var old = b.innerHTML;
      b.innerHTML = '测试中…';
      b.setAttribute('disabled', '');
      setTimeout(function () { b.innerHTML = old; b.removeAttribute('disabled'); }, 700);
    }
  });
})();
