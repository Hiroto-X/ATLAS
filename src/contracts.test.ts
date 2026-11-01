import { describe, it, expect } from "vitest";
import { SystemInfoSchema } from "./contracts";

describe("SystemInfoSchema", () => {
  it("accepts valid system information", () => {
    const result = {
      os: "windows",
      architecture: "x86_64",
      hostname: "ATLAS-PC",
      appVersion: "0.1.0",
    };

    expect(SystemInfoSchema.safeParse(result).success).toBe(true);
  });

  it("rejects missing properties", () => {
    const result = {
      os: "windows",
      architecture: "x86_64",
    };

    expect(SystemInfoSchema.safeParse(result).success).toBe(false);
  });

  it("rejects invalid property types", () => {
    const result = {
      os: "windows",
      architecture: "x86_64",
      hostname: 123,
      appVersion: "0.1.0",
    };

    expect(SystemInfoSchema.safeParse(result).success).toBe(false);
  });

  it("rejects empty strings", () => {
    const result = {
      os: "",
      architecture: "x86_64",
      hostname: "ATLAS-PC",
      appVersion: "0.1.0",
    };

    expect(SystemInfoSchema.safeParse(result).success).toBe(false);
  });
});