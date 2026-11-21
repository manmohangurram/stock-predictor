import { ref } from "vue";
import { api } from "./api";

const username = ref("");
let asked = false;

export function useAdmin() {
  if (!asked) {
    asked = true;
    api
      .me()
      .then((r) => (username.value = r.username))
      .catch(() => {});
  }
  return {
    username,
    setName(name: string): void {
      username.value = name;
    },
  };
}
