void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int main(void) {
    int x = 1;
    int **old = malloc(8);
    if (!old) return 1;
    *old = &x;
    int **pp = realloc(old, 8);
    if (!pp) { free(old); return 1; }
    int *p = *pp;
    free(pp);
    return p == 0;
}
