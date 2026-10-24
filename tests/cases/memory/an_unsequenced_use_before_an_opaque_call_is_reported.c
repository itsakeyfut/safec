void *malloc(int n);
int g(int a);
int h(int *q);
int f(void) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    *p = 1;
    int x = g(*p) + h(p);
    return x;
}
