<script lang="ts">
  import { tStore } from '$lib/i18n';
</script>

<h1>{$tStore('help.cr.h1')}</h1>
<p class="lead">{$tStore('help.cr.lead')}</p>

<section id="cr-what">
  <h3>{$tStore('help.cr.what.h')}</h3>
  <p>{$tStore('help.cr.what.p')}</p>
  <div class="callout">{$tStore('help.cr.what.callout')}</div>
</section>

<section id="cr-req">
  <h3>{$tStore('help.cr.req.h')}</h3>
  <p>{$tStore('help.cr.req.p')}</p>
  <p>{$tStore('help.cr.req.pair')}</p>
  <p>{$tStore('help.cr.req.close')}</p>
</section>

<section id="cr-peers">
  <h3>{$tStore('help.cr.peers.h')}</h3>
  <p>{$tStore('help.cr.peers.p')}</p>
  <p>{$tStore('help.cr.peers.kinds')}</p>
  <p>{$tStore('help.cr.peers.explicit')}</p>
</section>

<section id="cr-broadcast">
  <h3>{$tStore('help.cr.broadcast.h')}</h3>
  <p>{$tStore('help.cr.broadcast.announce')}</p>
  <p>{$tStore('help.cr.broadcast.codex')}</p>
</section>

<section id="cr-registry">
  <h3>{$tStore('help.cr.registry.h')}</h3>
  <p>{$tStore('help.cr.registry.p')}</p>
  <div class="callout">{$tStore('help.cr.registry.callout')}</div>
</section>
