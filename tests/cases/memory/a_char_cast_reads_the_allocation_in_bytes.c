void *malloc(unsigned long long n);
void free(void *p);
int main(void) {
    int *p = malloc(8);
    if (!p) {
        return 0;
    }
    char *c = (char *)p;
    c[7] = 1;
    c[8] = 2;
    free(p);
    return 0;
}
