typedef int I;
typedef int *P;
typedef int (*F)(void);
typedef struct S { int a; } T;
typedef unsigned long U;
I twice(I x) { return x + x; }
int main(void) {
    I a = 1;
    P p = &a;
    U u = 2;
    {
        int I = 3;
        a = a + I - 3;
    }
    I b = (I)u;
    I c = sizeof(I) == sizeof(int) ? 0 : 1;
    for (I i = 0; i < 2; i++) { a = a + 0; }
    return *p + b - 3 + c + twice(0);
}
