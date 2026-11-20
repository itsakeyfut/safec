void *malloc(int n);
void free(void *p);
int main(void) {
    goto first;
again:
    ;
    int *p;
    free(p);
    return 0;
first:
    p = malloc(4);
    goto again;
}
