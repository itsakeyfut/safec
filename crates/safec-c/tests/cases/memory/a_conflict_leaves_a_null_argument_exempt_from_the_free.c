void *realloc(void *p, int n);
void free(void *p);
int main(void) {
    int *p = 0;
    int *r = realloc(p, (p = 0, 4));
    free(r);
    return 0;
}
