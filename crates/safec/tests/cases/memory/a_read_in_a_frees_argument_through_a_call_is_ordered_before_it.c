void *malloc(int n);
void free(void *p);
int g(int a);
int f(void) {
    int *p = malloc(4);
    free(p + g(*p));
    return 0;
}
