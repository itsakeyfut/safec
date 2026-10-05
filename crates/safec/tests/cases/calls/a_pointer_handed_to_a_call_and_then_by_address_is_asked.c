void *malloc(int n);
void release(int *p);
int use2(int **pp);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    release(a);
    return use2(&a);
}
