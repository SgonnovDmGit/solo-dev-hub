<script lang="ts">
  import { tStore } from '$lib/i18n';
</script>

<h1>{$tStore('help.qs.h1')}</h1>
<p class="lead">{$tStore('help.qs.lead')}</p>

<section id="qs-what">
  <h3>{$tStore('help.qs.what.h')}</h3>
  <p>{$tStore('help.qs.what.p')}</p>
</section>

<section id="qs-steps">
  <h3>{$tStore('help.qs.steps.h')}</h3>
  <ol class="steps">
    <li>{$tStore('help.qs.steps.s1')}</li>
    <li>{$tStore('help.qs.steps.s2')}</li>
    <li>{$tStore('help.qs.steps.s3')}</li>
  </ol>
</section>

<section id="qs-map">
  <h3>{$tStore('help.qs.map.h')}</h3>
  <p>{$tStore('help.qs.map.p')}</p>
  <div class="callout">{$tStore('help.qs.map.callout')}</div>
</section>
