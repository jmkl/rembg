async (promptext) => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  const findComposer = () =>
    document.querySelector("#prompt-textarea") ||
    document.querySelector('div[contenteditable="true"][role="textbox"]');

  function insertPrompt(el, text) {
    el.focus();
    if (el instanceof HTMLTextAreaElement) {
      Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      ).set.call(el, text);
      el.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          inputType: "insertText",
          data: text,
        }),
      );
      return;
    }
    const s = getSelection(),
      r = document.createRange();
    r.selectNodeContents(el);
    s.removeAllRanges();
    s.addRange(r);
    if (!document.execCommand("insertText", false, text)) {
      el.textContent = text;
      el.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          inputType: "insertText",
          data: text,
        }),
      );
    }
  }
  function findButton() {
    return (
      document.querySelector('[data-testid="send-button"]') ||
      document.querySelector('[type="submit"]')
    );
  }

  async function sendPrompt(text, timeout = 30000) {
    const el = findComposer();
    if (!el) throw Error("composer not found");
    if (text.trim()) insertPrompt(el, text);
    await sleep(1000);
    for (
      const start = Date.now();
      Date.now() - start < timeout;
      await sleep(500)
    ) {
      const b = findButton();
      if (b && !b.disabled) return b.click();
    }
    throw Error("send button not ready");
  }

  sendPrompt(promptext).catch(console.error);
};
