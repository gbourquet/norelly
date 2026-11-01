<script lang="ts">
  // Squelette de la phase 1 : la mise en page en 3 panneaux arrive en phase 3.
  const appName = "Norelly";
</script>

<main>
  <h1>{appName}</h1>
</main>

<style>
  main {
    display: grid;
    place-items: center;
    height: 100vh;
  }

  h1 {
    font-weight: 500;
    color: var(--color-text-muted);
  }
</style>
