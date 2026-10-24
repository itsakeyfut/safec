void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(8);
    free(p);
    p[0];
    return 0;
}
