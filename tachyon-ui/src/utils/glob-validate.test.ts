import { describe, expect, it } from "vitest";
import { isValidGlob, isValidRoutingTuple } from "./glob-validate";

describe("isValidGlob", () => {
  it("accepts simple path globs", () => {
    expect(isValidGlob("db/prod/*")).toBe(true);
    expect(isValidGlob("https://api.example.com/**")).toBe(true);
    expect(isValidGlob("**")).toBe(true);
    expect(isValidGlob("secret-*")).toBe(true);
  });

  it("rejects empty or whitespace-only patterns", () => {
    expect(isValidGlob("")).toBe(false);
    expect(isValidGlob("   ")).toBe(false);
  });

  it("rejects patterns with bare leading/trailing whitespace", () => {
    expect(isValidGlob(" db/*")).toBe(false);
    expect(isValidGlob("db/* ")).toBe(false);
  });

  it("rejects unbalanced opening brace", () => {
    expect(isValidGlob("{non-balanced")).toBe(false);
    expect(isValidGlob("{a,{b}")).toBe(false);
  });

  it("rejects unbalanced closing brace", () => {
    expect(isValidGlob("non-balanced}")).toBe(false);
    expect(isValidGlob("{a},b}")).toBe(false);
  });

  it("accepts balanced braces", () => {
    expect(isValidGlob("{a,b,c}/*")).toBe(true);
    expect(isValidGlob("{x}/{y}")).toBe(true);
  });

  it("rejects embedded whitespace", () => {
    expect(isValidGlob("db/ prod/*")).toBe(false);
    expect(isValidGlob("a b")).toBe(false);
  });
});

describe("isValidRoutingTuple", () => {
  it("accepts valid routing tuples", () => {
    expect(isValidRoutingTuple("service-a -> service-b")).toBe(true);
    expect(isValidRoutingTuple("* -> target")).toBe(true);
  });

  it("rejects patterns without ' -> '", () => {
    expect(isValidRoutingTuple("service-a->service-b")).toBe(false);
    expect(isValidRoutingTuple("service-a - service-b")).toBe(false);
    expect(isValidRoutingTuple("service-a")).toBe(false);
  });

  it("inherits isValidGlob rejections", () => {
    expect(isValidRoutingTuple("")).toBe(false);
    expect(isValidRoutingTuple("{open -> dest")).toBe(false);
  });
});
