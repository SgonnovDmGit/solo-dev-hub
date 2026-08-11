<script lang="ts">
  import { tStore } from '$lib/i18n';
</script>

<h1>{$tStore('help.bg.h1')}</h1>
<p class="lead">{$tStore('help.bg.lead')}</p>

<section id="bg-flow">
  <h3>{$tStore('help.bg.flow.h')}</h3>
  <div class="flow">
    <span class="pill created">created</span><span class="arw">→</span>
    <span class="pill prog">in-progress</span><span class="arw">→</span>
    <span class="pill test">testing</span><span class="arw">→</span>
    <span class="pill ok">confirmed ✓</span><span class="arw">/</span>
    <span class="pill rej">rejected</span>
  </div>
  <p>{$tStore('help.bg.flow.p')}</p>
</section>

<section id="bg-edit">
  <h3>{$tStore('help.bg.edit.h')}</h3>
  <table class="edit">
    <thead>
      <tr>
        <th>{$tStore('help.bg.edit.col1')}</th>
        <th>{$tStore('help.bg.edit.col2')}</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td><code>status, comment</code></td>
        <td class="yes">{$tStore('help.bg.edit.r1who')}</td>
      </tr>
      <tr>
        <td><code>description, severity, category</code></td>
        <td class="no">{$tStore('help.bg.edit.r2who')}</td>
      </tr>
      <tr>
        <td><code>fix_attempts</code></td>
        <td class="no">{$tStore('help.bg.edit.r3who')}</td>
      </tr>
    </tbody>
  </table>
  <p>{$tStore('help.bg.edit.note')}</p>
</section>

<section id="bg-confirm">
  <h3>{$tStore('help.bg.confirm.h')}</h3>
  <p>{$tStore('help.bg.confirm.p')}</p>
  <div class="callout">{$tStore('help.bg.confirm.callout')}</div>
</section>
