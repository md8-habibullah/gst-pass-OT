/** True in the phone and tablet shells, where the app can go to the background and the camera plugin exists. */
export const isMobileShell = (): boolean => /Android|iPhone|iPad/i.test(navigator.userAgent);
