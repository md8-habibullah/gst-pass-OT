/** What a page's address looks like on screen. The website is the thing the person must recognise, so it is shown on its own. */
export interface OriginParts {
  /** `example.com`: the name without the scheme and port. */
  host: string;
  /** The port when it is not the usual one, else `null`. */
  port: string | null;
  /** `https`, or `http` for a page on this computer (localhost). */
  secure: boolean;
  /** This is a page on this computer (a developer's test page), not a website. */
  local: boolean;
}

export function originParts(origin: string): OriginParts {
  try {
    const u = new URL(origin);
    return { host: u.hostname, port: u.port || null, secure: u.protocol === "https:", local: u.hostname === "localhost" };
  } catch {
    return { host: origin, port: null, secure: false, local: false };
  }
}
