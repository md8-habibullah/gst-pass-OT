import { copyText } from "../../lib/clipboard";
import { app, pushToast } from "../../state/app";
import { PasswordGenerator } from "../items/PasswordGenerator";
import { PageBody, PageHeader } from "../shell/PageHeader";

/** The standalone generator (sidebar entry). Uses the same code as the one inside the editor. */
export function GeneratorView() {
  return (
    <PageBody>
      <PageHeader title="Generator" subtitle="Random passwords made on this device from your system's secure random source. Nothing is saved or sent anywhere." />
      <PasswordGenerator
        actionLabel="Copy password"
        onUse={async (pw) => {
          const clear = app.get().clipboardClear;
          try {
            await copyText(pw, clear);
            pushToast({ kind: "success", text: `Password copied. The clipboard clears in ${clear} seconds.`, countdownMs: clear * 1000 });
          } catch {
            pushToast({ kind: "error", text: "Copying was blocked. Select the password and copy it by hand." });
          }
        }}
      />
    </PageBody>
  );
}
