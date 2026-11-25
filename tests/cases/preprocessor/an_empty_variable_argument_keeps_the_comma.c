int count(int n, ...);
#define L(fmt, ...) count(fmt, ## __VA_ARGS__)

int v = L(1,);
