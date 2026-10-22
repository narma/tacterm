// Workflow concurrency serializes runs before this check. Only reuse a fully
// successful run of the same workflow, PR and synthetic merge commit (run-name).
function isSuccessfulDuplicate(candidate, current) {
  return candidate.id !== current.id
    && candidate.workflow_id === current.workflow_id
    && candidate.event === 'pull_request'
    && candidate.head_sha === current.head_sha
    && candidate.display_title === current.display_title
    && candidate.status === 'completed'
    && candidate.conclusion === 'success';
}

async function shouldRun({ github, context, core, validationJobs = [], runAttempt = Number(process.env.GITHUB_RUN_ATTEMPT) }) {
  // Explicit reruns and non-PR events must always execute their jobs.
  if (context.eventName !== 'pull_request' || runAttempt > 1) return true;
  // No declared validation jobs means there is no proof safe to reuse.
  if (validationJobs.length === 0) return true;
  try {
    const { data: current } = await github.rest.actions.getWorkflowRun({
      ...context.repo, run_id: context.runId,
    });
    for await (const { data } of github.paginate.iterator(github.rest.actions.listWorkflowRuns, {
      ...context.repo, workflow_id: current.workflow_id,
      event: 'pull_request', head_sha: current.head_sha, per_page: 100,
    })) {
      for (const duplicate of data.filter(run => isSuccessfulDuplicate(run, current))) {
        const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRun, {
          ...context.repo, run_id: duplicate.id, filter: 'latest', per_page: 100,
        });
        // A deduplicated workflow is also successful, but its validation jobs
        // are skipped. Never let that run become another reuse candidate.
        if (validationJobs.every(name => jobs.some(job => job.name === name
          && job.status === 'completed' && job.conclusion === 'success'))) {
          core.notice(`This PR revision already passed: ${duplicate.html_url}`);
          return false;
        }
      }
    }
  } catch (error) {
    // Losing the optimization must never prevent validation.
    core.warning(`Could not check previous CI runs; running validation: ${error.message}`);
  }
  return true;
}

module.exports = { isSuccessfulDuplicate, shouldRun };
