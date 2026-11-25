import { copyText } from "../../lib/clipboard";
import { app, pushToast } from "../../state/app";
import { PasswordGenerator } from "../items/PasswordGenerator";
import { PageBody, PageHeader } from "../shell/PageHeader";

/** The standalone generator (sidebar entry). Uses the same code as the one inside the editor. */
export function GeneratorView() {
  return (
    <PageBody>
      <PageHeader title="Generator" subtitle="Random passwords made on this device from your system's secure random source. A password is never saved. Your last settings are, inside your encrypted vault, and follow it to every device you sync." />
      <PasswordGenerator
        actionLabel="Copy password"
        onUse={async (pw) => {
          const clear = app.get().clipboardClear;
          try {
            await copyText(pw, clear);
            pushToast({ kind: "success", text: `Password copied. The clipboard clears in ${clear} seconds.` });
          } catch {
            pushToast({ kind: "error", text: "Copying was blocked. Select the password and copy it by hand." });
          }
        }}
      />
    </PageBody>
  );
}
