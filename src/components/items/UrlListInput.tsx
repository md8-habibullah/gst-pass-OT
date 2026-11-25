import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { joinUrls, MAX_URLS, urlsOf } from "../../lib/urls";
import { Button, IconButton } from "../ui/Button";
import { Field, TextInput } from "../ui/Inputs";

/**
 * The web addresses of a login: one box each, and a button for another. The value is the one `url` field, an address a line
 * (see lib/urls.ts), so everything else that reads that field keeps working. Boxes the person has not filled in yet are kept
 * on this screen only; they never reach the stored value. With one box it is the plain "Website" field; with several they
 * are "Website 1", "Website 2" and so on, each with its own label.
 */
export function UrlListInput({ label, value, onChange, placeholder }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string }) {
  const [rows, setRows] = useState<string[]>(() => {
    const urls = urlsOf(value);
    return urls.length ? urls : [""];
  });
  const set = (next: string[]) => {
    setRows(next);
    onChange(joinUrls(next));
  };
  const several = rows.length > 1;
  return (
    <div role="group" aria-label={`${label}s`} className="flex flex-col gap-2">
      {rows.map((row, i) => (
        <Field key={i} label={several ? `${label} ${i + 1}` : label}>
          {({ id, describedBy }) => (
            <div className="flex items-center gap-2">
              <TextInput
                id={id}
                aria-describedby={describedBy}
                type="url"
                inputMode="url"
                value={row}
                placeholder={i === 0 ? placeholder : "Another web address"}
                maxLength={2048}
                onChange={(e) => set(rows.map((r, j) => (j === i ? e.target.value : r)))}
              />
              {(several || row) && (
                <IconButton label={`Remove ${label.toLowerCase()}${several ? ` ${i + 1}` : ""}`} tone="danger" onClick={() => set(several ? rows.filter((_, j) => j !== i) : [""])}>
                  <Trash2 className="size-[18px]" />
                </IconButton>
              )}
            </div>
          )}
        </Field>
      ))}
      {rows.length < MAX_URLS && rows[rows.length - 1] !== "" && (
        <Button className="self-start" icon={<Plus className="size-4" />} onClick={() => setRows([...rows, ""])}>
          Add another {label.toLowerCase()}
        </Button>
      )}
    </div>
  );
}
