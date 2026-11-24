void *malloc(unsigned long long size);
void free(void *pointer);

struct Buffer {
    char tag;
    int items[];
};

struct Wide {
    char tag;
    long long items[];
};

int main(void) {
    int wrong = 0;
    // No bytes of its own: the struct is the `char` and the padding to the
    // elements' alignment.
    wrong += sizeof(struct Buffer) == sizeof(int) ? 0 : 1;
    // And as its element is aligned, which a `long long` is to 8 on every
    // host this runs on.
    wrong += sizeof(struct Wide) == 8 ? 0 : 1;
    struct Buffer *b = malloc(sizeof(struct Buffer) + 3 * sizeof(int));
    if (!b) {
        return 100;
    }
    b->tag = 'x';
    for (int i = 0; i < 3; i++) {
        b->items[i] = i * 10;
    }
    int *walk = b->items;
    wrong += walk[2] == 20 && b->items[1] == 10 && b->tag == 'x' ? 0 : 1;
    free(b);
    return wrong;
}
