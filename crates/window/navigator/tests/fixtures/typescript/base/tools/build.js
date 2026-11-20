import { readFileSync } from "node:fs";

export const manifest = JSON.parse(readFileSync("package.json", "utf8"));
