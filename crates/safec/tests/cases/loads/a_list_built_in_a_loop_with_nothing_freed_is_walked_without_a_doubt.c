void *malloc(int n);
void free(void *p);

int main(void) {
    void **head = 0;
    int i = 0;
    while (i < 3) {
        void **node = malloc(8);
        if (node == 0) {
            return 0;
        }
        *node = head;
        head = node;
        i = i + 1;
    }
    int count = 0;
    void **n = head;
    while (n != 0) {
        count = count + 1;
        n = *n;
    }
    return count;
}
