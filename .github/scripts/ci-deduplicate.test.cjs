const test = require('node:test');
const assert = require('node:assert/strict');
const { isSuccessfulDuplicate, shouldRun } = require('./ci-deduplicate.cjs');

const current = {
  id: 42, workflow_id: 7, event: 'pull_request', head_sha: 'head',
  display_title: 'Test / pull_request / 1 / merge-sha',
};
const passed = { ...current, id: 41, status: 'completed', conclusion: 'success' };

test('only a successful identical PR merge revision can replace a run', () => {
  assert.equal(isSuccessfulDuplicate(passed, current), true);
  for (const change of [
    { id: 42 }, { workflow_id: 8 }, { event: 'push' }, { head_sha: 'other' },
    { display_title: 'Test / pull_request / 2 / merge-sha' },
    { display_title: 'Test / pull_request / 1 / new-base-merge-sha' },
    { status: 'in_progress' }, { conclusion: 'failure' },
    { conclusion: 'cancelled' }, { conclusion: 'skipped' },
  ]) assert.equal(isSuccessfulDuplicate({ ...passed, ...change }, current), false);
});

function fixture(pages) {
  let calls = 0;
  const args = {
    runAttempt: 1,
    context: { eventName: 'pull_request', repo: { owner: 'owner', repo: 'repo' }, runId: 42 },
    core: { notice() {}, warning() {} },
    github: {
      rest: { actions: {
        async getWorkflowRun() { calls++; return { data: current }; },
        listWorkflowRuns() {},
      } },
      paginate: { async *iterator(_method, options) {
        assert.equal(options.head_sha, current.head_sha);
        assert.equal(options.workflow_id, current.workflow_id);
        for (const runs of pages) yield { data: runs };
      } },
    },
  };
  return { args, calls: () => calls };
}

test('finds a successful duplicate beyond the first API page', async () => {
  const { args } = fixture([[current], [passed]]);
  assert.equal(await shouldRun(args), false);
});

test('failed or cancelled previous runs do not suppress validation', async () => {
  const { args } = fixture([[{ ...passed, conclusion: 'failure' }, { ...passed, conclusion: 'cancelled' }]]);
  assert.equal(await shouldRun(args), true);
});

test('API failures fall back to running validation', async () => {
  const { args } = fixture([]);
  args.github.rest.actions.getWorkflowRun = async () => { throw new Error('API unavailable'); };
  let warning;
  args.core.warning = message => { warning = message; };
  assert.equal(await shouldRun(args), true);
  assert.match(warning, /API unavailable/);
});

test('manual reruns and non-PR events bypass deduplication', async () => {
  for (const [event, attempt] of [['pull_request', 2], ['push', 1], ['workflow_dispatch', 1]]) {
    const { args, calls } = fixture([[passed]]);
    args.context.eventName = event;
    args.runAttempt = attempt;
    assert.equal(await shouldRun(args), true);
    assert.equal(calls(), 0);
  }
});
