import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { previewBackend as be, PREVIEW_PASSWORD, seedPreview } from "./mock";
import type { ItemDraft } from "./types";

const login = (title: string, over: Partial<ItemDraft> = {}): ItemDraft => ({
  type: "login", title, favorite: false, reprompt: false, tags: [], folder: null, customFields: [],
  fields: { username: "me@example.com", password: "pw-1", url: "", notes: "" }, ...over,
});

// The preview backend follows the core's rules for folders, search and permanent delete
// (src-tauri/src/storage/vault_folders.rs, vault_search.rs, session.rs).
describe("preview backend: folders, search and permanent delete", () => {
  beforeAll(async () => { await seedPreview(); });
  beforeEach(async () => {
    const list = await be.listProfiles();
    await be.lock().catch(() => undefined);
    await be.selectProfile(list.profiles[0]!.id);
    await be.unlock(PREVIEW_PASSWORD);
  });

  it("makes, renames and removes folders; what was inside follows or moves up", async () => {
    const made = await be.createFolder("Work/Clients");
    expect(made.prefs.folders).toContain("Work/Clients");
    const item = await be.createItem(login("Acme", { folder: "Work/Clients" }));
    const renamed = await be.renameFolder("Work", "Job");
    expect(renamed.prefs.folders).toContain("Job/Clients");
    expect((await be.listItems()).find((i) => i.id === item.id)?.folder).toBe("Job/Clients");
    await be.removeFolder("Job/Clients");
    expect((await be.listItems()).find((i) => i.id === item.id)?.folder).toBe("Job");
  });

  it("refuses a folder that is too deep", async () => {
    await expect(be.createFolder("a/b/c/d/e/f/g")).rejects.toBeTruthy();
  });

  it("moves items into a folder and out again", async () => {
    const item = await be.createItem(login("Mover"));
    expect(await be.moveItems([item.id], "Elsewhere")).toBe(1);
    expect((await be.listItems()).find((i) => i.id === item.id)?.folder).toBe("Elsewhere");
    expect(await be.moveItems([item.id], "Elsewhere")).toBe(0);
    expect(await be.moveItems([item.id], null)).toBe(1);
  });

  it("keeps several web addresses a line each, tidied", async () => {
    const item = await be.createItem(login("Multi", { fields: { username: "u", password: "p", url: " https://a.example \n\nhttps://b.example\nhttps://a.example", notes: "" } }));
    const view = await be.getItem(item.id);
    expect(view.fields.find((f) => f.key === "url")?.value).toBe("https://a.example\nhttps://b.example");
  });

  it("searches notes and field names, never a password", async () => {
    const item = await be.createItem(login("Cabin", { fields: { username: "u", password: "zebra-pass", url: "", notes: "the alarm code is behind the clock" } }));
    expect((await be.searchItems("alarm clock", false)).find((h) => h.id === item.id)?.matched).toContain("Notes");
    expect((await be.searchItems("zebra-pass", false)).some((h) => h.id === item.id)).toBe(false);
  });

  it("deleting forever needs the password and, for a recent trashing, the sum", async () => {
    const item = await be.createItem(login("Doomed"));
    await be.trashItem(item.id);
    const plan = await be.purgePlan(item.id);
    expect(plan.recent).toBe(true);
    expect(plan.sum).toBeTruthy();
    await expect(be.purgeItem(item.id, "not the password", "1")).rejects.toMatchObject({ code: "wrong_password" });
    await expect(be.purgeItem(item.id, PREVIEW_PASSWORD, "-1")).rejects.toMatchObject({ code: "wrong_answer" });
    // the wrong answer used the sum up: a new one is needed
    const again = await be.purgePlan(item.id);
    expect(again.sum).toBeTruthy();
    const answer = String(Function(`"use strict"; return (${again.sum})`)());
    await be.purgeItem(item.id, PREVIEW_PASSWORD, answer);
    expect((await be.listTrash()).some((t) => t.id === item.id)).toBe(false);
  });
});
