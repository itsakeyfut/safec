#define OBJ __VA_ARGS__
#define FN(x) __VA_ARGS__
#define OPT(x) __VA_OPT__(x)
#define UNCLOSED(...) __VA_OPT__(a
#define NESTED(...) __VA_OPT__(__VA_OPT__(a))
#define BESIDE(a, ...) a ## __VA_OPT__(b)
#define AFTER(a, ...) __VA_OPT__(b) ## a
#define TWO(a, b, ...) a
#define ONLY(...) f(0, ## __VA_ARGS__)
#define NO_OPEN(...) __VA_OPT__ x
#define NAMED_COMMA(y, ...) (0 , ## y)
int v = __VA_ARGS__;
int w = TWO(1);
int x = ONLY();
int u = __VA_OPT__;
int n = NAMED_COMMA(1);
