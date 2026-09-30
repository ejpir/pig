import { randomUUID } from 'node:crypto';

export function createUi(output, theme) {
  const pending = new Map();
  const emit = (method, fields = {}) => output({ type: 'extension_ui_request', id: randomUUID(), method, ...fields });
  const dialog = (method, fields, opts = {}, fallback, parse) => {
    if (opts.signal?.aborted) return Promise.resolve(fallback);
    const id = randomUUID();
    return new Promise(resolve => {
      let timer;
      const finish = value => {
        if (!pending.delete(id)) return;
        clearTimeout(timer);
        opts.signal?.removeEventListener('abort', abort);
        resolve(value);
      };
      const abort = () => {
        if (pending.has(id)) output({ type: 'extension_ui_cancel', id });
        finish(fallback);
      };
      pending.set(id, response => finish(response.cancelled ? fallback : parse(response)));
      opts.signal?.addEventListener('abort', abort, { once: true });
      if (opts.timeout > 0) timer = setTimeout(abort, opts.timeout);
      output({ type: 'extension_ui_request', id, method, ...fields, timeout: opts.timeout });
    });
  };
  const ui = {
    select: (title, options, opts) => dialog('select', { title, options }, opts, undefined,
      response => options.includes(response.value) ? response.value : undefined),
    confirm: (title, message, opts) => dialog('confirm', { title, message }, opts, false,
      response => response.confirmed === true),
    input: (title, placeholder, opts) => dialog('input', { title, placeholder }, opts, undefined,
      response => typeof response.value === 'string' ? response.value : undefined),
    editor: (title, prefill) => dialog('editor', { title, prefill }, {}, undefined,
      response => typeof response.value === 'string' ? response.value : undefined),
    notify: (message, notifyType) => emit('notify', { message, notifyType }),
    setStatus: (statusKey, statusText) => emit('setStatus', { statusKey, statusText }),
    setWidget: (widgetKey, widgetLines, options) => {
      if (widgetLines === undefined || Array.isArray(widgetLines)) {
        emit('setWidget', { widgetKey, widgetLines, widgetPlacement: options?.placement });
      }
    },
    setTitle: title => emit('setTitle', { title }),
    setEditorText: text => emit('set_editor_text', { text }),
    pasteToEditor: text => emit('set_editor_text', { text }),
    getEditorText: () => '', getEditorComponent: () => undefined,
    custom: async () => undefined, onTerminalInput: () => () => {},
    setWorkingMessage() {}, setWorkingVisible() {}, setWorkingIndicator() {},
    setHiddenThinkingLabel() {}, setFooter() {}, setHeader() {}, setToolsExpanded() {},
    addAutocompleteProvider() {}, setEditorComponent() {},
    theme, getAllThemes: () => [], getTheme: () => undefined, getToolsExpanded: () => false,
    setTheme: () => ({ success: false, error: 'Theme switching not supported in RPC mode' }),
  };
  return {
    ui,
    respond: response => pending.get(response.id)?.(response),
    cancel: () => { for (const [id, respond] of [...pending.entries()]) {
      output({ type: 'extension_ui_cancel', id }); respond({ cancelled: true });
    } },
  };
}
