chars = {
    '0': [6,9,9,9,6],
    '2': [6,9,2,4,15],
    '5': [15,8,14,1,14],
    '7': [15,2,4,8,8],
    '.': [0,0,0,0,6],
    'k': [9,10,12,10,9],
    'H': [9,9,15,9,9],
    'z': [15,2,4,8,15]
}
for c, rows in chars.items():
    val = 0
    for i, r in enumerate(rows):
        val |= (r << (i * 4))
    print(f"let char_{c} = {val}u;")
