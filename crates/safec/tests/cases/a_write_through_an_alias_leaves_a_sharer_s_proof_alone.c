void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int *s = p;
    int **pp = &s;
    free(p);
    *pp = 0;
    *p = 1;
    return 0;
}
