<script lang="ts">
  import { tStore } from '$lib/i18n';
</script>

<h1>{$tStore('help.bg.h1')}</h1>
<p class="lead">{$tStore('help.bg.lead')}</p>

<section id="bg-flow">
  <h3>{$tStore('help.bg.flow.h')}</h3>
  <div class="flow">
    <span class="pill created">{$tStore('status.created')}</span><span class="arw">→</span>
    <span class="pill prog">{$tStore('status.in-progress')}</span><span class="arw">→</span>
    <span class="pill test">{$tStore('status.testing')}</span><span class="arw">→</span>
    <span class="pill ok">{$tStore('status.confirmed')} ✓</span><span class="arw">/</span>
    <span class="pill rej">{$tStore('status.rejected')}</span>
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
        <td>{$tStore('help.bg.edit.r1field')}</td>
        <td class="yes">{$tStore('help.bg.edit.r1who')}</td>
      </tr>
      <tr>
        <td>{$tStore('help.bg.edit.r2field')}</td>
        <td class="no">{$tStore('help.bg.edit.r2who')}</td>
      </tr>
      <tr>
        <td>{$tStore('help.bg.edit.r3field')}</td>
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
