#define OBJ __VA_ARGS__
#define FN(x) __VA_ARGS__
#define OPT(x) __VA_OPT__(x)
#define UNCLOSED(...) __VA_OPT__(a
#define NESTED(...) __VA_OPT__(__VA_OPT__(a))
#define BESIDE(a, ...) a ## __VA_OPT__(b)
#define TWO(a, b, ...) a
#define ONLY(...) f(0, ## __VA_ARGS__)
int v = __VA_ARGS__;
int w = TWO(1);
int x = ONLY();
