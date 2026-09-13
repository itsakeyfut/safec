void *malloc(int n);
void free(void *p);
int f(int i) {
    int *p = malloc(4);
    free(p);
    *p, i;
    return 0;
}
