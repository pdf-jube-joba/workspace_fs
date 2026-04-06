import katex from "katex";
import {transformMathMarkdown} from "./katex_pre.js";

export default function createKatexTransform() {
  return async function transform(text, context = {}) {
    return transformMathMarkdown(text, {
      macros: context.macros ?? {},
      renderToString(value, options) {
        return katex.renderToString(value, options);
      },
    });
  };
}
