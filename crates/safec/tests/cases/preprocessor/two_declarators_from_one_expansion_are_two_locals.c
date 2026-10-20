void *malloc(int n);
void free(void *p);
#define DECL int *a = malloc(4), *b = 0;

int main(void) {
    DECL
    if (!a) return 0;
    free(a);
    b = malloc(4);
    if (!b) return 0;
    *a = 1;
    free(b);
    return 0;
}
