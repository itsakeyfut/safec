#define ONE(a) a
#define TWO(a, b) a

int two = TWO(1);
int three = ONE(1, 2);
int inside = ONE(
#define Z 1
2);
int open = ONE(1 + 2;
