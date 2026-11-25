// A login can have several web addresses. They live in its one `url` field, a line each (the core tidies them the same
// way: trimmed, no blanks, no repeats, at most 20), so every other part of the app keeps treating `url` as one text field.

export const MAX_URLS = 20;

/** The addresses in a `url` field value: one per line, trimmed, none empty, no repeats. */
export function urlsOf(value: string | null | undefined): string[] {
  const out: string[] = [];
  for (const line of (value ?? "").split(/\r?\n/)) {
    const u = line.trim();
    if (u && !out.includes(u)) out.push(u);
  }
  return out;
}

export const joinUrls = (urls: readonly string[]): string => urlsOf(urls.join("\n")).join("\n");

/** The first address, which is the one a row shows and a plain "open" uses. */
export const firstUrl = (value: string | null | undefined): string => urlsOf(value)[0] ?? "";

/** An address as it should be opened: a bare `example.com` gets `https://`. Anything with a scheme is left as it is. */
export function openable(url: string): string {
  const u = url.trim();
  return /^[a-z][a-z0-9+.-]*:\/\//i.test(u) ? u : `https://${u}`;
}

/** How an address is shown in a list of them: no scheme, no `www.`, no trailing slash. */
export function shortUrl(url: string): string {
  return url.trim().replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").replace(/^www\./i, "").replace(/\/$/, "");
}
