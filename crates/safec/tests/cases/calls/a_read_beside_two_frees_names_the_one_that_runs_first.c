void free(void *p);
void *malloc(int n);
int g(int a);
int f(void) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    *p = 1;
    int *q = p;
    int x = g(*p) + (free((0, free(q), p)), 0);
    return x;
}
