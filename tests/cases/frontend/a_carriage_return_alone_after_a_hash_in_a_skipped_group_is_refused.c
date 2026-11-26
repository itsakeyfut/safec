void *malloc(unsigned long long);
void free(void *);

int main(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    free(p);
#if 0
#else
    return 0;
#endif
    free(p);
    return 0;
}
