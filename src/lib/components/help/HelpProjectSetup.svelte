<script lang="ts">
  import { tStore } from '$lib/i18n';
</script>

<h1>{$tStore('help.pr.h1')}</h1>
<p class="lead">{$tStore('help.pr.lead')}</p>

<section id="pr-structure">
  <h3>{$tStore('help.pr.structure.h')}</h3>
  <p>{$tStore('help.pr.structure.p')}</p>
</section>

<section id="pr-roles">
  <h3>{$tStore('help.pr.roles.h')}</h3>
  <ul>
    <li>{$tStore('help.pr.roles.client')}</li>
    <li>{$tStore('help.pr.roles.server')}</li>
    <li>{$tStore('help.pr.roles.ms')}</li>
    <li>{$tStore('help.pr.roles.tool')}</li>
  </ul>
</section>

<section id="pr-workspace">
  <h3>{$tStore('help.pr.workspace.h')}</h3>
  <p>{$tStore('help.pr.workspace.p')}</p>
  <div class="callout">{$tStore('help.pr.workspace.callout')}</div>
</section>
