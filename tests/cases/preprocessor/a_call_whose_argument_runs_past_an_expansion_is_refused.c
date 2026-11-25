#define STR(x) #x
#define XSTR(x) STR(x)
#define OPEN XSTR(a
#define USE OPEN  b)
const char *s = USE;
#define F(x) ((x) + 1)
#define ID(x) x
#define OPEN2 ID(1 + F
#define USE2 OPEN2 (1))
int t = USE2;
#define K(a, b) a
#define F2(x) K(x)
#define OPEN3 ID(F2
int u = OPEN3 (1));
