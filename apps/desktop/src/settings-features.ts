// Settings → Features: turn feature flags on or off (see features.ts).

import { el } from "./dom";
import { enabled, FLAGS, isOverridden, setEnabled, type FlagStage } from "./features";
import { registerSettingsSection } from "./settings-dialog";

const STAGE_NOTE: Record<FlagStage, string> = {
  experimental: "Experimental: off unless you turn it on, and it may change.",
  beta: "Beta: on for everyone; tell us if something goes wrong.",
  stable: "Stable: this switch goes away in the next release.",
};

registerSettingsSection({
  id: "features",
  title: "Features",
  render(body, rerender) {
    const reset = el("button", { type: "button" }, "Use the defaults");
    reset.disabled = !FLAGS.some((f) => isOverridden(f.id));
    reset.onclick = () => {
      for (const f of FLAGS) setEnabled(f.id, null);
      rerender();
    };
    body.append(
      el("h3", {}, "Features"),
      el(
        "p",
        { className: "muted" },
        "Newer features can be turned off here if they cause trouble on your system. Changes apply to new tabs and dialogs.",
      ),
      ...FLAGS.map((f) => {
        const toggle = el("input", { type: "checkbox", checked: enabled(f.id) });
        toggle.onchange = () => {
          setEnabled(f.id, toggle.checked);
          rerender();
        };
        return el(
          "div",
          { className: "pack-card flag-card" },
          el(
            "label",
            { className: "pack-head" },
            toggle,
            el("span", { className: "pack-title" }, el("strong", {}, f.title), el("small", { className: "muted" }, f.id)),
          ),
          el("p", { className: "muted pack-desc" }, f.description),
          el(
            "div",
            { className: "pack-facts" },
            el("span", { className: `badge stage-${f.stage}`, title: STAGE_NOTE[f.stage] }, f.stage),
            ...(isOverridden(f.id) ? [el("span", { className: "badge changed" }, "changed by you")] : []),
          ),
        );
      }),
      el("div", { className: "pack-install" }, reset),
    );
  },
});
