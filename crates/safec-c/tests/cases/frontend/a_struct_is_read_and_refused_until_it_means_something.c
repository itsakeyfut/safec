struct S { int x; char *p; };
struct S;
struct T s;
int f(struct S *p) {
    struct S q;
    return p->x + q.x;
}
int g(void) {
    int k = 1;
    return k;
}
int h(struct S *a, struct S *b) {
    a = b;
    return 0;
}
struct D { int a, b; } *d;
struct { int x; } anonymous;
struct S many[3];
struct S make(void);
int (*takes)(struct S *);
int abstract(int (struct S *));
int i(void) {
    struct U;
    struct E { int e; } *e;
    for (struct S *r = 0; 0;) {
    }
    many[0].x;
    make().x;
    return 0;
}
