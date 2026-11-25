#define M(z) z(M,
#define h(x, y) x(k) y)
#define k(x, y) (y), x)
int (*M)(int);
int k = 0;
int main(void) { return (int)sizeof( M(h) + 1 ); }
#define j(x, y) x + y
#define OPEN j(OPEN,
int w = sizeof(OPEN 2));
