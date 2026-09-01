<script lang="ts">
  import { tStore } from '$lib/i18n';
</script>

<h1>{$tStore('help.llm.h1')}</h1>
<p class="lead">{$tStore('help.llm.lead')}</p>

<section id="llm-files">
  <h3>{$tStore('help.llm.files.h')}</h3>
  <p>{$tStore('help.llm.files.p')}</p>
  <table class="edit">
    <thead>
      <tr>
        <th>{$tStore('help.llm.files.col1')}</th>
        <th>{$tStore('help.llm.files.col2')}</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td><code>docs/todo.md</code>&nbsp;· <code>docs/done.md</code></td>
        <td>{$tStore('help.llm.files.r1')}</td>
      </tr>
      <tr>
        <td><code>docs/bug-reports.md</code></td>
        <td>{$tStore('help.llm.files.r2')}</td>
      </tr>
      <tr>
        <td><code>docs/project.md</code></td>
        <td>{$tStore('help.llm.files.r3')}</td>
      </tr>
      <tr>
        <td><code>docs/sdh_skills/</code></td>
        <td>{$tStore('help.llm.files.r4')}</td>
      </tr>
      <tr>
        <td><code>CLAUDE.md</code></td>
        <td>{$tStore('help.llm.files.r5')}</td>
      </tr>
    </tbody>
  </table>
</section>

<section id="llm-bugs">
  <h3>{$tStore('help.llm.bugs.h')}</h3>
  <p>{$tStore('help.llm.bugs.p')}</p>
  <div class="callout">{$tStore('help.llm.bugs.callout')}</div>
</section>

<section id="llm-rules">
  <h3>{$tStore('help.llm.rules.h')}</h3>
  <p>{$tStore('help.llm.rules.p')}</p>
  <div class="callout">{$tStore('help.llm.rules.callout')}</div>
</section>

<section id="llm-optin">
  <h3>{$tStore('help.llm.optin.h')}</h3>
  <p>{$tStore('help.llm.optin.p')}</p>
  <div class="callout">{$tStore('help.llm.optin.callout')}</div>
</section>
