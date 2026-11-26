void *malloc(unsigned long long);
void free(void *);
#define B 1
int main(void) {
    char *p = malloc(4);
    if (!p) return 0;
#ifdef A
#elifdef B
    free(p);
#endif
#ifndef B
#elifndef A
    free(p);
#endif
    free(p);
    return 0;
}
