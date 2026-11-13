// Tested not null first, so on the arm that reaches `realloc` it is
// not (#931).
void *realloc(void *p, int n);
void free(void *p);
int g(int **old) {
    if (!old) return 0;
    int **pp = realloc(old, 8);
    if (!pp) { free(old); return 1; }
    int *p = *pp;
    free(pp);
    return p != 0;
}
