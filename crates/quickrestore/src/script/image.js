async (url, name) => {
  const m = url.match(/^data:(image\/[\w+.-]+);base64,(.+)$/s);
  if (!m) throw new Error("Invalid image data URL");
  const bytes = Uint8Array.from(atob(m[2].replace(/\s/g, "")), (c) =>
    c.charCodeAt(0),
  );
  const file = new File([bytes], name || "image." + m[1].split("/")[1], {
    type: m[1],
  });

  const find = () => {
    const i = [...document.querySelectorAll('input[type="file"]')];
    return (
      i.find((x) => /image/i.test(x.accept)) || i.find((x) => !x.accept) || i[0]
    );
  };

  let input = find();
  if (!input) {
    [...document.querySelectorAll("button")]
      .find(
        (b) =>
          b.getClientRects().length &&
          /attach|upload|add photos|add files/i.test(
            [b.getAttribute("aria-label"), b.dataset.testid, b.innerText].join(
              " ",
            ),
          ),
      )
      ?.click();
    for (let t = 0; t < 50 && !(input = find()); t++)
      await new Promise((r) => setTimeout(r, 100));
    if (!input) throw new Error("File input not found");
  }

  const dt = new DataTransfer();
  dt.items.add(file);
  input.files = dt.files;
  ["input", "change"].forEach((e) =>
    input.dispatchEvent(new Event(e, { bubbles: true })),
  );
};
