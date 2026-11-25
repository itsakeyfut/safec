#define NOPARAM(x) # y
#define FIRST(x) ## x
#define LAST(x) x ##
#define OBJ ## a
#define P(a, b) a ## b
#define HASH_OBJ # a
int v = P(+, -);
int main(void) { return 0; }
