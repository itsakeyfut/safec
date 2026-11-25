void *malloc(unsigned long n);
void free(void *p);

#line 100 "elsewhere.c"
int main(void) {
    int *p = malloc(4);
    free(p);
    free(p);
    return 0;
}
