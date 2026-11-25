// A Vitest reporter that appends one line to the consumers record for each
// case as it finishes, so that a process that is killed leaves the cases it
// had finished (research/investigations/consumers/):
//
//   FILE::DESCRIBE > NAME<TAB>STATUS
//
// FILE is the test file relative to the project, STATUS is pass, fail or skip.
// run.sh loads it with `vitest run --reporter=.../vitest_record.mjs FILE` and
// names the record in CASE_RECORD.
import { appendFileSync } from "node:fs";

const STATUS = { passed: "pass", failed: "fail", skipped: "skip" };

export default class CaseRecord {
  constructor() {
    this.path = process.env.CASE_RECORD;
    if (!this.path) throw new Error("CASE_RECORD names no file");
  }

  onTestCaseResult(testCase) {
    const name = `${testCase.module.relativeModuleId}::${testCase.fullName}`.replace(/[\t\r\n]+/g, " ");
    const status = STATUS[testCase.result().state] ?? "error";
    // One write of a whole line, so that a kill leaves no half line.
    appendFileSync(this.path, `${name}\t${status}\n`);
  }
}
