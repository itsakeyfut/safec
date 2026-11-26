void *malloc(unsigned long long);
void free(void *);

int main(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    free(p);
#define NOTE 1    free(p);
    return 0;
}
