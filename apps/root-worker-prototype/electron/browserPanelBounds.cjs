function normalizeBrowserBoundsUpdate(bounds, currentSequence = null) {
  const sequence = normalizeBrowserBoundsSequence(bounds?.sequence);
  if (
    sequence !== null &&
    currentSequence !== null &&
    sequence < currentSequence
  ) {
    return {
      apply: false,
      bounds: null,
      sequence: currentSequence,
    };
  }

  return {
    apply: true,
    bounds: {
      x: Math.max(0, Math.round(Number(bounds?.x) || 0)),
      y: Math.max(0, Math.round(Number(bounds?.y) || 0)),
      width: Math.max(0, Math.round(Number(bounds?.width) || 0)),
      height: Math.max(0, Math.round(Number(bounds?.height) || 0)),
    },
    sequence: sequence ?? currentSequence,
  };
}

function normalizeBrowserBoundsSequence(sequence) {
  const value = Number(sequence);
  return Number.isSafeInteger(value) && value > 0 ? value : null;
}

module.exports = {
  normalizeBrowserBoundsUpdate,
};
