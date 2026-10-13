// Lets Node run the app's TypeScript modules as they are: their imports leave out the
// `.ts` the bundler adds.
import { registerHooks } from "node:module";

registerHooks({
  resolve(specifier, context, next) {
    if (specifier.startsWith(".") && !/\.[cm]?[jt]s$/.test(specifier)) {
      try {
        return next(specifier + ".ts", context);
      } catch {
        // Not a TypeScript module: resolve it as written.
      }
    }
    return next(specifier, context);
  },
});
