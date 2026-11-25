#define D1(x) x x
#define D2(x) D1(D1(x))
#define D4(x) D2(D2(x))
#define D8(x) D4(D4(x))
#define D16(x) D8(D8(x))
int v[] = { D16(D4(D2(1))) };
