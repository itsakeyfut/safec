void *malloc(unsigned long long, ...);
void free(void *p);

int main(void) {
    char *q = malloc(4);
    if (!q)
        return 0;
    free(q);
    char *r = malloc(4, q);
    if (r)
        free(r);
    return 0;
}
