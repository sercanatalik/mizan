/**
 * Categorical palette in fixed order (blue, orange, aqua, yellow, magenta,
 * green, violet, red), validated on the light surface with the dataviz
 * validator: lightness band, chroma floor, adjacent CVD separation and the
 * normal-vision floor all pass. Three slots sit below 3:1 against the surface,
 * so charts using it always carry a legend and a table view. A ninth series is
 * never a new hue: it folds into "Other".
 */
export const CATEGORICAL = ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#6250d6", "#e34948"] as const;

export function slotColor(i: number): string {
  return CATEGORICAL[Math.min(i, CATEGORICAL.length - 1)]!;
}
