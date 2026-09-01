<script lang="ts">
  import { tStore } from '$lib/i18n';
</script>

<h1>{$tStore('help.dp.h1')}</h1>
<p class="lead">{$tStore('help.dp.lead')}</p>

<section id="dp-envs">
  <h3>{$tStore('help.dp.envs.h')}</h3>
  <p>{$tStore('help.dp.envs.p')}</p>
  <p>{$tStore('help.dp.envs.files')}</p>
</section>

<section id="dp-secrets">
  <h3>{$tStore('help.dp.secrets.h')}</h3>
  <p>{$tStore('help.dp.secrets.p')}</p>
  <div class="flow">
    <span class="pill prog">build</span><span class="arw">·</span>
    <span class="pill test">deploy</span><span class="arw">·</span>
    <span class="pill ok">runtime</span>
  </div>
  <p>{$tStore('help.dp.secrets.flags')}</p>
</section>

<section id="dp-bundles">
  <h3>{$tStore('help.dp.bundles.h')}</h3>
  <p>{$tStore('help.dp.bundles.p')}</p>
  <div class="callout">{$tStore('help.dp.bundles.crypto')}</div>
</section>

<section id="dp-report">
  <h3>{$tStore('help.dp.report.h')}</h3>
  <p>{$tStore('help.dp.report.p')}</p>
</section>
