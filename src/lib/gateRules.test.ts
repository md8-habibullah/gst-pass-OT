import { describe, expect, it } from "vitest";
import { credentialProblem, isPalindrome, panicProblems, reverseOf } from "./gateRules";

describe("credentialProblem", () => {
  it("needs at least four characters in every mode", () => {
    expect(credentialProblem("pin", "123")).toMatch(/at least 4 digits/);
    expect(credentialProblem("password", "abc")).toMatch(/at least 4 characters/);
    expect(credentialProblem("grid", "125")).toMatch(/at least 4 dots/);
    expect(credentialProblem("pin", "1234")).toBeNull();
    expect(credentialProblem("password", "a b!")).toBeNull();
    expect(credentialProblem("grid", "1258")).toBeNull();
  });
  it("keeps each mode's alphabet", () => {
    expect(credentialProblem("pin", "12a4")).toMatch(/digits only/);
    expect(credentialProblem("grid", "1230")).toMatch(/1 to 9/);
    expect(credentialProblem("grid", "1213")).toMatch(/same dot twice/);
  });
  it("counts characters, not UTF-16 units", () => {
    expect(credentialProblem("password", "😀😀😀")).toMatch(/at least 4/);
    expect(credentialProblem("password", "😀😀😀😀")).toBeNull();
  });
});

describe("panic rules", () => {
  it("rejects a palindrome only while reverse panic is on", () => {
    expect(isPalindrome("12321")).toBe(true);
    expect(isPalindrome("12345")).toBe(false);
    expect(panicProblems("pin", "12321", true, "").reverse).toMatch(/same backwards/);
    expect(panicProblems("pin", "12321", false, "").reverse).toBeNull();
    expect(reverseOf("13579")).toBe("97531");
  });
  it("keeps the custom panic code distinct from the passcode and its reverse", () => {
    expect(panicProblems("pin", "13579", true, "13579").panic).toMatch(/different/);
    expect(panicProblems("pin", "13579", true, "97531").panic).toMatch(/reversed/);
    expect(panicProblems("pin", "13579", false, "97531").panic).toBeNull();
    expect(panicProblems("pin", "13579", true, "246").panic).toMatch(/panic code needs at least 4/);
    expect(panicProblems("pin", "13579", true, "2468").panic).toBeNull();
  });
});
