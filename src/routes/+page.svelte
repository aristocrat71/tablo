<script lang="ts">
  import { onMount } from "svelte";
  import Avatar from "$lib/Avatar.svelte";
  import Panel from "$lib/Panel.svelte";
  import Dashboard from "$lib/Dashboard.svelte";
  import Toast from "$lib/Toast.svelte";
  import { currentLabel } from "$lib/bridge";
  import { initStore } from "$lib/state.svelte";

  let surface = $state<string>("");

  onMount(() => {
    surface = currentLabel();
    document.body.dataset.surface = surface;
    initStore();
  });
</script>

{#if surface === "avatar"}
  <Avatar />
{:else if surface === "panel"}
  <Panel />
{:else if surface === "dashboard"}
  <Dashboard />
{:else if surface === "toast"}
  <Toast />
{/if}
