void *malloc(int n);
void free(void *p);
int f(int a) {
    int *p = malloc(4);
    int x = (a ? *p : 0) + (free(p), 0);
    return x;
}
