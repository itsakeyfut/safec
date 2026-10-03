void *malloc(int n);
int g(int a);
int h(int *q);
int f(void) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    *p = 1;
    int y = g(*p);
    int x = h(p) + 1;
    return x + y;
}
