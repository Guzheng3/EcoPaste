import {
  defineConfig,
  presetIcons,
  presetWind4,
  transformerDirectives,
  transformerVariantGroup,
} from "unocss";
import { presetAntdColors } from "./src/unocss/presetAntdColors";

export default defineConfig({
  presets: [
    presetWind4(),
    presetAntdColors(),
    presetIcons({
      collections: {
        "lets-icons": () =>
          import("@iconify-json/lets-icons/icons.json").then((m) => m.default),
        lucide: () =>
          import("@iconify-json/lucide/icons.json").then((m) => m.default),
        ph: () => import("@iconify-json/ph/icons.json").then((m) => m.default),
      },
    }),
  ],
  transformers: [
    transformerVariantGroup(),
    transformerDirectives({
      applyVariable: ["--uno"],
    }),
  ],
});
