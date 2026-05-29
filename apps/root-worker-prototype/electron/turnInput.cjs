function buildTurnInput(payload) {
  const input = [];

  for (const skill of payload.skills ?? []) {
    if (!skill?.name || !skill?.path) {
      continue;
    }
    input.push({
      type: "skill",
      name: skill.name,
      path: skill.path,
    });
  }

  if (payload.text.trim()) {
    input.push({
      type: "text",
      text: payload.text.trim(),
      text_elements: [],
    });
  }

  for (const image of payload.images ?? []) {
    if (!image?.dataUrl) {
      continue;
    }
    input.push({
      type: "image",
      url: image.dataUrl,
    });
  }

  return input;
}

module.exports = {
  buildTurnInput,
};
