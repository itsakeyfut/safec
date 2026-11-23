void *malloc(unsigned long long n);
void free(void *p);
typedef int *P;
int main(void) {
    P p = malloc(sizeof(int));
    if (!p) {
        return 1;
    }
    *p = 0;
    free(p);
    return *p;
}
