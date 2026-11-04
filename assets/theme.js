// The remembered theme, applied before the first paint so nothing flashes. A file rather than an
// inline script so the pages can run under a Content Security Policy that allows only 'self'.
try {
  var t = localStorage.getItem('nbb-theme');
  if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
} catch (e) {}
