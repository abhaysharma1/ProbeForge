import { describe, expect, it } from "bun:test";
import { RepositoryIngestError, type RepositoryIngestErrorCode } from "./errors";
import { parseTreeInventory } from "./tree";

const SHA = "a".repeat(40);

function record(mode: string, type: string, path: string, size: string): string {
  return `${mode} ${type} ${SHA} ${size}\t${path}\0`;
}

function expectCode(callback: () => unknown, code: RepositoryIngestErrorCode): void {
  try {
    callback();
    throw new Error("expected an error");
  } catch (error) {
    expect(error).toBeInstanceOf(RepositoryIngestError);
    expect((error as RepositoryIngestError).code).toBe(code);
  }
}

describe("parseTreeInventory", () => {
  it("parses blobs, symlinks, and submodules", () => {
    const output =
      record("100644", "blob", "src/app.ts", "12") +
      record("100755", "blob", "run.sh", "10") +
      record("120000", "blob", "escape-link", "15") +
      record("160000", "commit", "vendor/example", "-");
    const tree = parseTreeInventory(output, {
      maxFiles: 10,
      maxTotalBytes: 100,
      maxDepth: 5,
    });
    expect(tree.entries).toHaveLength(4);
    expect(tree.fileCount).toBe(3);
    expect(tree.totalBytes).toBe(37);
    expect(tree.symlinks).toEqual(["escape-link"]);
    expect(tree.submodules).toEqual(["vendor/example"]);
    expect(tree.entries.find((entry) => entry.path === "src/app.ts")?.kind).toBe("blob");
  });

  it("enforces file, byte, and depth limits", () => {
    expectCode(
      () =>
        parseTreeInventory(
          record("100644", "blob", "a.ts", "1") + record("100644", "blob", "b.ts", "1"),
          {
            maxFiles: 1,
            maxTotalBytes: 100,
            maxDepth: 5,
          },
        ),
      "too_many_files",
    );
    expectCode(
      () =>
        parseTreeInventory(record("100644", "blob", "a.ts", "100"), {
          maxFiles: 10,
          maxTotalBytes: 99,
          maxDepth: 5,
        }),
      "repository_too_large",
    );
    expectCode(
      () =>
        parseTreeInventory(record("100644", "blob", "a/b/c/d.ts", "1"), {
          maxFiles: 10,
          maxTotalBytes: 100,
          maxDepth: 3,
        }),
      "repository_too_deep",
    );
  });

  it("rejects unsafe paths and unsupported entries", () => {
    for (const path of [
      "../escape.ts",
      "/absolute.ts",
      "C:/absolute.ts",
      ".git/config",
      "a\\b.ts",
      "a//b.ts",
      "src/file.ts:stream",
      "CON",
      "aux.txt",
      "trailing.",
      "trailing ",
    ]) {
      expectCode(
        () =>
          parseTreeInventory(record("100644", "blob", path, "1"), {
            maxFiles: 10,
            maxTotalBytes: 100,
            maxDepth: 10,
          }),
        "unsafe_path",
      );
    }
    expectCode(
      () =>
        parseTreeInventory(record("040000", "tree", "src", "-"), {
          maxFiles: 10,
          maxTotalBytes: 100,
          maxDepth: 10,
        }),
      "unsupported_tree_entry",
    );
  });

  it("rejects content beneath a symlink or submodule", () => {
    expectCode(
      () =>
        parseTreeInventory(
          record("120000", "blob", "link", "1") + record("100644", "blob", "link/escape.ts", "1"),
          { maxFiles: 10, maxTotalBytes: 100, maxDepth: 10 },
        ),
      "unsafe_path",
    );
    expectCode(
      () =>
        parseTreeInventory(
          record("160000", "commit", "vendor/example", "-") +
            record("100644", "blob", "vendor/example/file.ts", "1"),
          { maxFiles: 10, maxTotalBytes: 100, maxDepth: 10 },
        ),
      "unsafe_path",
    );
  });
});
