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

const validatedJobs = [{ name: 'build', status: 'completed', conclusion: 'success' }];

function fixture(pages, jobsByRun = { [passed.id]: validatedJobs }) {
  let calls = 0;
  const checkedRuns = [];
  const paginate = async (method, options) => {
    assert.equal(method, args.github.rest.actions.listJobsForWorkflowRun);
    assert.equal(options.filter, 'latest');
    checkedRuns.push(options.run_id);
    return jobsByRun[options.run_id] || [];
  };
  paginate.iterator = async function* (_method, options) {
    assert.equal(options.head_sha, current.head_sha);
    assert.equal(options.workflow_id, current.workflow_id);
    for (const runs of pages) yield { data: runs };
  };
  const args = {
    validationJobs: ['build'],
    runAttempt: 1,
    context: { eventName: 'pull_request', repo: { owner: 'owner', repo: 'repo' }, runId: 42 },
    core: { notice() {}, warning() {} },
    github: {
      rest: { actions: {
        async getWorkflowRun() { calls++; return { data: current }; },
        listWorkflowRuns() {},
        listJobsForWorkflowRun() {},
      } },
      paginate,
    },
  };
  return { args, calls: () => calls, checkedRuns };
}

test('a deduplicated run cannot replace the original validated run', async () => {
  const a = { ...passed, id: 39 };
  const b = passed;
  // B reuses A. C must look past B and still prove that A ran validation.
  assert.equal(await shouldRun(fixture([[a]], { [a.id]: validatedJobs }).args), false);
  const skipped = [{ name: 'build', status: 'completed', conclusion: 'skipped' }];
  const { args, checkedRuns } = fixture([[b], [a]], {
    [a.id]: validatedJobs, [b.id]: skipped,
  });
  assert.equal(await shouldRun(args), false);
  assert.deepEqual(checkedRuns, [b.id, a.id]);
  assert.equal(await shouldRun(fixture([[b]], { [b.id]: skipped }).args), true);
});

test('all declared validation jobs must have completed successfully', async () => {
  for (const jobs of [[],
    [{ name: 'build', status: 'completed', conclusion: 'skipped' }],
    [{ name: 'build', status: 'completed', conclusion: 'failure' }],
    [{ name: 'build', status: 'in_progress', conclusion: null }],
    [{ name: 'Check for duplicate CI', status: 'completed', conclusion: 'success' }],
  ]) {
    assert.equal(await shouldRun(fixture([[passed]], { [passed.id]: jobs }).args), true);
  }
  const { args } = fixture([[passed]]);
  args.validationJobs = ['build', 'another matrix job'];
  assert.equal(await shouldRun(args), true);
  args.validationJobs = [];
  assert.equal(await shouldRun(args), true);
});

test('matrix validation requires every platform, but not a PR-excluded deployment', async () => {
  const jobs = [
    { name: 'Build (Linux)', status: 'completed', conclusion: 'success' },
    { name: 'Build (Windows)', status: 'completed', conclusion: 'success' },
    { name: 'deploy', status: 'completed', conclusion: 'skipped' },
  ];
  const { args } = fixture([[passed]], { [passed.id]: jobs });
  args.validationJobs = ['Build (Linux)', 'Build (Windows)'];
  assert.equal(await shouldRun(args), false);
  jobs[1].conclusion = 'skipped';
  assert.equal(await shouldRun(args), true);
});

test('job lookup errors do not suppress validation', async () => {
  const { args } = fixture([[passed]]);
  const iterator = args.github.paginate.iterator;
  args.github.paginate = async () => { throw new Error('job lookup unavailable'); };
  args.github.paginate.iterator = iterator;
  let warning;
  args.core.warning = message => { warning = message; };
  assert.equal(await shouldRun(args), true);
  assert.match(warning, /job lookup unavailable/);
});

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
