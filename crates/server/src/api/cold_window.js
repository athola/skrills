// Defense in depth: server already html-escapes every user-derived
// string, and the browser swaps fragments via DOMParser +
// replaceChildren. DOMParser parses <script> tags into nodes that do
// NOT execute when later attached to the document, so even if the
// server-side escape ever regresses, an injected payload can't run.
const evt = new EventSource('/dashboard.sse');
const swap = (id, html) => {
  const el = document.getElementById(id);
  if (!el) return;
  const parsed = new DOMParser().parseFromString(html, 'text/html');
  el.replaceChildren(...parsed.body.childNodes);
};
evt.addEventListener('alert',    e => swap('alert-body',    e.data));
evt.addEventListener('hint',     e => swap('hint-body',     e.data));
evt.addEventListener('research', e => swap('research-body', e.data));
evt.addEventListener('status',   e => swap('status-bar',    e.data));
evt.onerror = () => swap('status-bar',
  '<span class="severity-warning">reconnecting…</span>');
